use std::sync::Arc;

use anyhow::{Context, bail};
use server::{
    AppState,
    auth::{
        otp::{LogOtpSender, OtpSender},
        password::{blocking, hash_password, validate_password},
        routes::{create_owner_account, validate_display_name},
        sms::SmsOtpSender,
        sms_termii::TermiiSms,
    },
    build_app,
    chat::hub::ChatHub,
    config::Config,
    couriers::shipbubble::ShipbubbleCourier,
    crypto::FieldCipher,
    escrow::jobs,
    market::layout::Layout,
    moderation::{
        claude::ClaudeTone,
        tone::{NoTone, ToneChecker},
    },
    notify::{
        fcm::FcmPush,
        push::{NoPush, PushSender},
        worker as push_worker,
    },
    payments::{ProviderRegistry, mono::MonoProvider},
    phone::normalize_ng_phone,
    pool_options,
    uploads::{
        s3::S3Store,
        store::{NoStore, ObjectStore},
    },
};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

/// How long open connections get to finish after a shutdown signal.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug)]
enum Command {
    /// `server` or `server serve`
    Serve,
    /// `server create-owner --phone <phone> --name <name>` (flags kept as given)
    CreateOwner(Vec<String>),
}

fn parse_command(args: &[String]) -> anyhow::Result<Command> {
    match args.split_first() {
        None => Ok(Command::Serve),
        Some((cmd, rest)) if cmd == "serve" => {
            if !rest.is_empty() {
                bail!("serve takes no arguments");
            }
            Ok(Command::Serve)
        }
        Some((cmd, rest)) if cmd == "create-owner" => Ok(Command::CreateOwner(rest.to_vec())),
        // The value is not echoed: it could be anything a user typed.
        Some(_) => bail!("unknown command (expected: serve, create-owner, art)"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The art tool needs no database and none of the server's other settings.
    if args.first().map(String::as_str) == Some("art") {
        return server::art::run(&args[1..]).await;
    }
    let command = parse_command(&args)?;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    let sms = sms_sender(&config)?;
    // Before connecting or migrating: a production server with no payment
    // provider would fail every checkout. Admin commands still run.
    if matches!(command, Command::Serve) {
        require_payment_provider(
            &config.app_env,
            config.mono.is_some() || config.paystack.is_some(),
        )?;
        require_moderation(&config.app_env, config.claude.is_some())?;
        require_push(&config.app_env, config.fcm.is_some())?;
        require_uploads(&config.app_env, config.s3.is_some())?;
        require_sms(&config.app_env, sms.is_some())?;
        require_identity(
            &config.app_env,
            config.identity_provider.is_some() && config.nin_index_key.is_some(),
        )?;
    }
    // Before connecting: a broken layout is a deploy mistake, so name it and stop.
    let market = Arc::new(Layout::load(&config.market_layout_dir).with_context(|| {
        format!(
            "market layout in {} is not usable",
            config.market_layout_dir.display()
        )
    })?);
    let db = pool_options(10).connect(&config.database_url).await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    let mut providers = ProviderRegistry::new();
    match config.mono.clone() {
        Some(mono_cfg) => {
            let live = mono_cfg.live;
            providers.register(Arc::new(MonoProvider::new(mono_cfg)?));
            tracing::info!(live, "mono payment provider registered");
        }
        None => tracing::warn!("MONO_SECRET_KEY not set: Mono is not configured"),
    }

    match config.paystack.clone() {
        Some(ps_cfg) => {
            let live = ps_cfg.live;
            providers.register(Arc::new(server::payments::paystack::PaystackProvider::new(
                ps_cfg,
            )?));
            tracing::info!(live, "paystack payment provider registered");
        }
        None => tracing::warn!("PAYSTACK_SECRET_KEY not set: Paystack is not configured"),
    }

    let courier: Option<Arc<dyn server::couriers::CourierProvider>> =
        match config.shipbubble.clone() {
            Some(sb_cfg) => {
                let live = sb_cfg.live;
                let courier = ShipbubbleCourier::new(sb_cfg)?;
                tracing::info!(live, "shipbubble courier provider registered");
                Some(Arc::new(courier))
            }
            None => {
                tracing::warn!("SHIPBUBBLE_API_KEY not set: courier delivery is unavailable");
                None
            }
        };

    let chat_hub = Arc::new(ChatHub::new());
    let tone: Arc<dyn ToneChecker> = match config.claude.clone() {
        Some(cfg) => Arc::new(ClaudeTone::new(cfg)?),
        None if std::env::var("TONE_PROVIDER").as_deref() == Ok("fake") => {
            tracing::warn!("TONE_PROVIDER=fake: tone checks are scripted, not real");
            Arc::new(server::moderation::fake::ScriptedTone)
        }
        None => {
            tracing::warn!("ANTHROPIC_API_KEY not set: tone checks pass everything");
            Arc::new(NoTone)
        }
    };

    let push: Arc<dyn PushSender> = match config.fcm.clone() {
        Some(cfg) => Arc::new(FcmPush::new(cfg)?),
        None => {
            tracing::warn!("FCM_SERVICE_ACCOUNT_FILE not set: pushes are accepted and dropped");
            Arc::new(NoPush)
        }
    };

    let objects: Arc<dyn ObjectStore> = match config.s3.clone() {
        Some(cfg) => Arc::new(S3Store::new(cfg)?),
        None => {
            tracing::warn!("S3_* not set: photo uploads are unavailable");
            Arc::new(NoStore)
        }
    };

    let identity: Option<Arc<dyn server::identity::IdentityProvider>> = match config
        .identity_provider
        .as_deref()
    {
        Some("fake") => {
            tracing::warn!("IDENTITY_PROVIDER=fake: identity checks are scripted, not real");
            Some(Arc::new(server::identity::fake::FakeIdentity::new()))
        }
        _ => {
            tracing::warn!("IDENTITY_PROVIDER not set: the seller identity check is unavailable");
            None
        }
    };

    // A partner is optional: with none configured the gateway answers "not configured"
    // without a network call, and nothing else in the server changes.
    let partner = match config.partner.clone() {
        Some(cfg) => {
            let secret = cfg.webhook_secret.clone();
            tracing::info!("partner service configured");
            server::partners::PartnerGateway::new(
                Arc::new(server::partners::client::HttpPartner::new(cfg)?),
                db.clone(),
                Some(secret),
            )
        }
        None => server::partners::PartnerGateway::new(
            Arc::new(server::partners::client::NoPartner),
            db.clone(),
            None,
        ),
    };

    let state = AppState {
        db,
        cipher: Arc::new(FieldCipher::new(config.field_key)),
        index_key: Arc::new(config.index_key),
        otp: sms.unwrap_or_else(|| {
            tracing::warn!("SMS_PROVIDER not set: codes are written to the log, not sent");
            Arc::new(LogOtpSender {
                echo: config.otp_dev_echo,
            })
        }),
        providers: Arc::new(providers),
        payment_redirect_url: config.public_redirect_url.clone(),
        verify_gate: Arc::new(server::payments::VerifyGate::default()),
        // REVISIT(nipost): swap in a real adapter once NIPOST's API is documented.
        postcodes: Arc::new(server::delivery::postcode::ManualPostcode),
        courier,
        tone,
        push,
        objects,
        chat_hub: chat_hub.clone(),
        // REVISIT(mono-statement): install an adapter once Mono's statement API is confirmed.
        statements: None,
        app_config: Arc::new(config.app.clone()),
        market,
        identity,
        nin_key: config.nin_index_key.map(Arc::new),
        partner: Arc::new(partner),
    };

    if matches!(command, Command::Serve) {
        let placed = server::market::placement::place_missing(&state.db, &state.market).await?;
        tracing::info!(placed, "stores placed on the market map");
    }

    match command {
        Command::Serve => serve(state, &config).await,
        Command::CreateOwner(flags) => create_owner(state, &flags).await,
    }
}

async fn serve(state: AppState, config: &Config) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "listening");
    let db = state.db.clone();
    // Background jobs (Task 9): stopped by the same shutdown signal, and
    // awaited before the pool closes.
    let stop_jobs = CancellationToken::new();
    let jobs_loop = tokio::spawn(jobs::run_loop(state.clone(), stop_jobs.clone()));
    // The push loop stops on the same token: between ticks, never mid-send.
    let push_loop = tokio::spawn(push_worker::run_loop(state.clone(), stop_jobs.clone()));
    let (signalled_tx, signalled_rx) = tokio::sync::oneshot::channel::<()>();
    let cancel_jobs = stop_jobs.clone();
    let chat_hub = state.chat_hub.clone();
    let server = axum::serve(listener, build_app(state)).with_graceful_shutdown(async move {
        shutdown_signal().await;
        tracing::info!("shutdown signal received, draining connections");
        cancel_jobs.cancel();
        chat_hub.stop();
        let _ = signalled_tx.send(());
    });
    // Idle keep-alive connections can stall the drain forever, so bound it once the signal fires.
    let served = tokio::select! {
        res = server => res,
        _ = async {
            if signalled_rx.await.is_ok() {
                tokio::time::sleep(SHUTDOWN_GRACE).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {
            tracing::warn!("drain timed out, shutting down with connections still open");
            Ok(())
        }
    };
    // Also on a server error: the jobs stop between rows, never mid-call.
    stop_jobs.cancel();
    if tokio::time::timeout(SHUTDOWN_GRACE, jobs_loop)
        .await
        .is_err()
    {
        tracing::warn!("jobs did not stop in time, shutting down anyway");
    }
    if tokio::time::timeout(SHUTDOWN_GRACE, push_loop)
        .await
        .is_err()
    {
        tracing::warn!("pushes did not stop in time, shutting down anyway");
    }
    db.close().await;
    tracing::info!("shut down");
    served?;
    Ok(())
}

/// Resolves on Ctrl-C or (on unix) SIGTERM so in-flight requests can finish.
async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

/// Production refuses to start without a payment provider: every checkout
/// would fail and webhooks would 404.
fn require_payment_provider(app_env: &str, configured: bool) -> anyhow::Result<()> {
    if app_env == "production" && !configured {
        bail!(
            "APP_ENV is production but no payment provider is configured (set MONO_SECRET_KEY or PAYSTACK_SECRET_KEY)"
        );
    }
    Ok(())
}

/// Production refuses to start without photo storage: nobody could list a
/// product with a photo.
fn require_uploads(app_env: &str, configured: bool) -> anyhow::Result<()> {
    if app_env == "production" && !configured {
        bail!(
            "APP_ENV is production but photo storage is not configured (set the S3_* variables and UPLOADS_PUBLIC_BASE_URL)"
        );
    }
    Ok(())
}

/// Builds the SMS sender for `SMS_PROVIDER` (Termii, decision D4); `None` when unset.
fn sms_sender(config: &Config) -> anyhow::Result<Option<Arc<dyn OtpSender>>> {
    match config.termii.clone() {
        Some(cfg) => {
            tracing::info!("termii SMS provider registered");
            Ok(Some(Arc::new(SmsOtpSender::new(Arc::new(TermiiSms::new(
                cfg,
            )?)))))
        }
        None => Ok(None),
    }
}

/// Production refuses to start without an SMS sender: nobody could sign up on a real
/// phone, and the log would hold their codes.
fn require_sms(app_env: &str, configured: bool) -> anyhow::Result<()> {
    if app_env == "production" && !configured {
        bail!("APP_ENV is production but no SMS sender is configured (set SMS_PROVIDER)");
    }
    Ok(())
}

/// Production refuses to start without an identity provider: no seller could be verified,
/// so no store could take orders.
fn require_identity(app_env: &str, configured: bool) -> anyhow::Result<()> {
    if app_env == "production" && !configured {
        bail!(
            "APP_ENV is production but no identity provider is configured (set IDENTITY_PROVIDER and NIN_INDEX_KEY)"
        );
    }
    Ok(())
}

/// Production refuses to start without push: buyers and sellers would miss
/// every notice unless they happened to open the inbox.
fn require_push(app_env: &str, fcm_configured: bool) -> anyhow::Result<()> {
    if app_env == "production" && !fcm_configured {
        bail!("APP_ENV is production but push is not configured (set FCM_SERVICE_ACCOUNT_FILE)");
    }
    Ok(())
}

/// Production refuses to start without a tone check: chat would deliver
/// messages nobody checked.
fn require_moderation(app_env: &str, claude_configured: bool) -> anyhow::Result<()> {
    if app_env == "production" && !claude_configured {
        bail!("APP_ENV is production but no tone check is configured (set ANTHROPIC_API_KEY)");
    }
    Ok(())
}

fn flag(args: &[String], name: &str) -> anyhow::Result<String> {
    let pos = args
        .iter()
        .position(|a| a == name)
        .with_context(|| format!("missing {name}"))?;
    args.get(pos + 1)
        .cloned()
        .with_context(|| format!("missing value for {name}"))
}

/// `server create-owner --phone <phone> --name <name>`; password is read without echo.
async fn create_owner(state: AppState, args: &[String]) -> anyhow::Result<()> {
    let phone = normalize_ng_phone(&flag(args, "--phone")?).map_err(|e| anyhow::anyhow!("{e}"))?;
    let name = validate_display_name(&flag(args, "--name")?).map_err(|e| anyhow::anyhow!("{e}"))?;
    let password = rpassword::prompt_password("Password: ")?;
    validate_password(&password).map_err(|e| anyhow::anyhow!("{e}"))?;

    let hash = blocking(move || hash_password(&password))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let id = create_owner_account(&state, &phone, &name, &hash)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_argument_and_serve_both_start_the_server() {
        assert!(matches!(parse_command(&args(&[])).unwrap(), Command::Serve));
        assert!(matches!(
            parse_command(&args(&["serve"])).unwrap(),
            Command::Serve
        ));
    }

    #[test]
    fn create_owner_keeps_its_flags() {
        let cmd = parse_command(&args(&["create-owner", "--phone", "080", "--name", "N"])).unwrap();
        let Command::CreateOwner(rest) = cmd else {
            panic!("expected create-owner")
        };
        assert_eq!(rest, args(&["--phone", "080", "--name", "N"]));
    }

    #[test]
    fn production_requires_a_payment_provider() {
        let err = require_payment_provider("production", false).unwrap_err();
        assert!(err.to_string().contains("MONO_SECRET_KEY"), "{err}");
        assert!(err.to_string().contains("PAYSTACK_SECRET_KEY"), "{err}");
        assert!(require_payment_provider("production", true).is_ok());
        for env in ["development", "test", "staging"] {
            assert!(require_payment_provider(env, false).is_ok(), "{env}");
        }
    }

    #[test]
    fn production_requires_anthropic_key() {
        let err = require_moderation("production", false).unwrap_err();
        assert!(err.to_string().contains("ANTHROPIC_API_KEY"), "{err}");
        assert!(require_moderation("production", true).is_ok());
        assert!(require_moderation("development", false).is_ok());
    }

    #[test]
    fn production_requires_photo_storage() {
        let err = require_uploads("production", false).unwrap_err();
        assert!(err.to_string().contains("S3_"), "{err}");
        assert!(require_uploads("production", true).is_ok());
        for env in ["development", "test", "staging"] {
            assert!(require_uploads(env, false).is_ok(), "{env}");
        }
    }

    #[test]
    fn production_requires_an_sms_sender() {
        let err = require_sms("production", false).unwrap_err();
        assert!(err.to_string().contains("SMS_PROVIDER"), "{err}");
        assert!(require_sms("production", true).is_ok());
        for env in ["development", "test", "staging"] {
            assert!(require_sms(env, false).is_ok(), "{env}");
        }
    }

    #[test]
    fn production_requires_fcm() {
        let err = require_push("production", false).unwrap_err();
        assert!(
            err.to_string().contains("FCM_SERVICE_ACCOUNT_FILE"),
            "{err}"
        );
        assert!(require_push("production", true).is_ok());
        for env in ["development", "test", "staging"] {
            assert!(require_push(env, false).is_ok(), "{env}");
        }
    }

    #[test]
    fn unknown_command_and_extra_serve_args_rejected() {
        assert!(parse_command(&args(&["bogus"])).is_err());
        assert!(parse_command(&args(&["serve", "now"])).is_err());
    }
}
