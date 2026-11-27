use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::auth::sms_termii::{self, TermiiConfig};
use crate::couriers::shipbubble::{self, ShipbubbleConfig};
use crate::moderation::claude::{self, ClaudeConfig};
use crate::notify::fcm::FcmConfig;
use crate::payments::{
    mono::{self, AmountUnit, MonoConfig, Secret},
    paystack::{self, PaystackConfig},
};
use crate::uploads::s3::{self, S3Config};

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub app_env: String,
    pub field_key: [u8; 32],
    pub index_key: [u8; 32],
    pub otp_dev_echo: bool,
    /// Where the payment provider sends the buyer after paying (`PUBLIC_REDIRECT_URL`).
    /// Fixed by configuration, never taken from a request, and carries no personal data.
    pub public_redirect_url: String,
    /// Mono settings; `None` when `MONO_SECRET_KEY` is unset (Mono is then not
    /// registered and nothing else about Mono is required).
    pub mono: Option<MonoConfig>,
    /// Paystack settings; `None` when `PAYSTACK_SECRET_KEY` is unset. Either Mono or Paystack
    /// (or both) satisfies production's need for a payment provider.
    pub paystack: Option<PaystackConfig>,
    /// Shipbubble settings; `None` when `SHIPBUBBLE_API_KEY` is unset (courier
    /// delivery is then unavailable; the server still starts).
    pub shipbubble: Option<ShipbubbleConfig>,
    /// Claude settings; `None` when `ANTHROPIC_API_KEY` is unset (production
    /// then refuses to serve).
    pub claude: Option<ClaudeConfig>,
    /// FCM settings; `None` when `FCM_SERVICE_ACCOUNT_FILE` is unset
    /// (production then refuses to serve; elsewhere pushes are dropped).
    pub fcm: Option<FcmConfig>,
    /// Photo storage settings; `None` when none of the `S3_*` variables are set
    /// (production then refuses to serve; elsewhere uploads answer 503).
    pub s3: Option<S3Config>,
    /// The partner service; `None` when none of the `PARTNER_*` variables are set (it is optional).
    pub partner: Option<crate::partners::PartnerConfig>,
    /// Which SMS provider sends sign-up and reset codes (`SMS_PROVIDER`); `None` when unset
    /// (production then refuses to serve; elsewhere codes go to the log).
    pub sms_provider: Option<String>,
    /// Termii settings; `Some` exactly when `SMS_PROVIDER=termii`.
    pub termii: Option<TermiiConfig>,
    /// What the app is told at start-up: minimum versions, links, a maintenance notice.
    pub app: crate::app_config::AppClientConfig,
    /// Which identity provider checks sellers (`IDENTITY_PROVIDER`); `None` when unset
    /// (production then refuses to serve; elsewhere the check answers 503).
    pub identity_provider: Option<String>,
    /// Key of the NIN fingerprint (`NIN_INDEX_KEY`). Never rotate it: every verified
    /// seller would have to verify again.
    pub nin_index_key: Option<[u8; 32]>,
    /// Folder holding the market `layout.json` and `rooms/` (`MARKET_LAYOUT_DIR`).
    pub market_layout_dir: std::path::PathBuf,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("app_env", &self.app_env)
            .field("mono", &self.mono)
            .field("paystack", &self.paystack)
            .field("shipbubble", &self.shipbubble)
            .field("claude", &self.claude)
            .field("fcm", &self.fcm)
            .field("s3", &self.s3)
            .field("sms_provider", &self.sms_provider)
            .field("termii", &self.termii)
            .finish_non_exhaustive()
    }
}

/// Decodes a standard-base64 string that must be exactly 32 bytes.
fn key_from_b64(name: &str, raw: &str) -> Result<[u8; 32]> {
    let bytes = STANDARD
        .decode(raw.trim())
        .with_context(|| format!("{name} must be base64"))?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("{name} must decode to exactly 32 bytes"))
}

/// Environments in which OTP codes may be echoed to stdout.
const DEV_ECHO_ENVS: &[&str] = &["development", "test"];

/// `PUBLIC_REDIRECT_URL` default, used only in development and test.
pub const DEV_REDIRECT_URL: &str = "http://localhost:8080/checkout/return";

/// The payment return URL: optional (with a localhost default) in development and
/// test, required and https everywhere else.
fn redirect_url(raw: Option<String>, dev: bool) -> Result<String> {
    let url = match raw.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
        Some(url) => url,
        None if dev => return Ok(DEV_REDIRECT_URL.into()),
        None => bail!("PUBLIC_REDIRECT_URL is required outside development and test"),
    };
    let ok = if dev {
        url.starts_with("https://") || url.starts_with("http://")
    } else {
        url.starts_with("https://")
    };
    if !ok || url.contains(char::is_whitespace) || url.contains('?') {
        bail!("PUBLIC_REDIRECT_URL must be an https URL without a query string");
    }
    Ok(url)
}

/// Mono's settings, read only when `MONO_SECRET_KEY` is set. Live mode is
/// `APP_ENV=production`. Startup refuses to run when:
/// - `MONO_WEBHOOK_SECRET` is missing;
/// - `MONO_BASE_URL` is not https;
/// - `MONO_DIRECTPAY_AMOUNT_UNIT` (default `kobo`) or
///   `MONO_DISBURSE_AMOUNT_UNIT` is set to anything but `kobo` / `naira`;
/// - in live mode: the key does not start with `live_sk_`, or `MONO_DISBURSE_AMOUNT_UNIT`,
///   `MONO_DISBURSE_SOURCE_ACCOUNT` or `MONO_PAYOUT_RECIPIENT_EMAIL` is missing;
/// - outside live mode: the key is a `live_sk_` key.
///
/// Outside live mode the Disburse settings may be left out; payouts are then
/// `NotSupported` (manual). Errors name the variable, never its value.
/// Paystack: `PAYSTACK_SECRET_KEY` switches it on (`sk_live_` in production, `sk_test_`
/// elsewhere, a mismatch refused). The webhook signature uses the same key. Errors name the
/// variable, never its value.
fn paystack_config(
    var: &impl Fn(&str) -> Option<String>,
    live: bool,
) -> Result<Option<PaystackConfig>> {
    let get = |name: &str| {
        var(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let Some(secret_key) = get("PAYSTACK_SECRET_KEY") else {
        return Ok(None);
    };
    // Fail closed: production accepts only a key that says it is live, and a live key is
    // refused anywhere else.
    if live && !secret_key.starts_with("sk_live_") {
        bail!("PAYSTACK_SECRET_KEY must be a live key (sk_live_) when APP_ENV is production");
    }
    if !live && secret_key.starts_with("sk_live_") {
        bail!("PAYSTACK_SECRET_KEY is a live key but APP_ENV is not production");
    }
    if !secret_key.starts_with("sk_") || secret_key.contains(char::is_whitespace) {
        bail!("PAYSTACK_SECRET_KEY is not a Paystack secret key (it starts with sk_)");
    }
    let base_url = get("PAYSTACK_BASE_URL").unwrap_or_else(|| paystack::DEFAULT_BASE_URL.into());
    if !base_url.starts_with("https://") || base_url.contains(char::is_whitespace) {
        bail!("PAYSTACK_BASE_URL must be an https URL");
    }
    // Buyers have no email (they sign up with a phone), but Paystack needs one: a synthetic
    // address on a domain we own. Required in production, so receipts do not go to a stranger.
    let payer_email_domain = match get("PAYSTACK_PAYER_EMAIL_DOMAIN") {
        Some(d) => d.to_ascii_lowercase(),
        None if live => bail!("PAYSTACK_PAYER_EMAIL_DOMAIN is required in production"),
        None => "payer.invalid".to_string(),
    };
    let domain_ok = payer_email_domain.contains('.')
        && payer_email_domain.len() <= 100
        && !payer_email_domain.starts_with(['.', '-'])
        && !payer_email_domain.ends_with(['.', '-'])
        && payer_email_domain
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
    if !domain_ok {
        bail!("PAYSTACK_PAYER_EMAIL_DOMAIN must be a domain name such as pay.example.ng");
    }
    Ok(Some(PaystackConfig {
        secret_key: Secret::new(&secret_key),
        base_url,
        live,
        payer_email_domain,
        connect_timeout: paystack::CONNECT_TIMEOUT,
        timeout: paystack::REQUEST_TIMEOUT,
        insecure_http_for_tests: false,
    }))
}

fn mono_config(var: &impl Fn(&str) -> Option<String>, live: bool) -> Result<Option<MonoConfig>> {
    let get = |name: &str| {
        var(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let Some(secret_key) = get("MONO_SECRET_KEY") else {
        return Ok(None);
    };
    // Fail closed: production accepts only a key that says it is live.
    if live && !secret_key.starts_with("live_sk_") {
        bail!("MONO_SECRET_KEY must be a live key (live_sk_) when APP_ENV is production");
    }
    if !live && secret_key.starts_with("live_sk_") {
        bail!("MONO_SECRET_KEY is a live key but APP_ENV is not production");
    }
    let webhook_secret = get("MONO_WEBHOOK_SECRET")
        .context("MONO_WEBHOOK_SECRET is required when MONO_SECRET_KEY is set")?;
    let base_url = get("MONO_BASE_URL").unwrap_or_else(|| mono::DEFAULT_BASE_URL.into());
    if !base_url.starts_with("https://") || base_url.contains(char::is_whitespace) {
        bail!("MONO_BASE_URL must be an https URL");
    }
    let unit = |name: &str| -> Result<Option<AmountUnit>> {
        match get(name) {
            None => Ok(None),
            Some(raw) => AmountUnit::parse(&raw)
                .map(Some)
                .with_context(|| format!("{name} must be kobo or naira")),
        }
    };
    let directpay_unit = unit("MONO_DIRECTPAY_AMOUNT_UNIT")?.unwrap_or(AmountUnit::Kobo);
    let disburse_unit = unit("MONO_DISBURSE_AMOUNT_UNIT")?;
    let disburse_source_account = get("MONO_DISBURSE_SOURCE_ACCOUNT");
    let payout_recipient_email = get("MONO_PAYOUT_RECIPIENT_EMAIL");
    if let Some(email) = &payout_recipient_email {
        let valid = !email.contains(char::is_whitespace)
            && email.split_once('@').is_some_and(|(local, domain)| {
                !local.is_empty() && domain.contains('.') && !domain.contains('@')
            });
        if !valid {
            bail!("MONO_PAYOUT_RECIPIENT_EMAIL must be an email address");
        }
    }
    if live {
        if disburse_unit.is_none() {
            bail!("MONO_DISBURSE_AMOUNT_UNIT is required in production (kobo or naira)");
        }
        if disburse_source_account.is_none() {
            bail!("MONO_DISBURSE_SOURCE_ACCOUNT is required in production");
        }
        if payout_recipient_email.is_none() {
            bail!("MONO_PAYOUT_RECIPIENT_EMAIL is required in production");
        }
    }
    Ok(Some(MonoConfig {
        secret_key: Secret::new(&secret_key),
        webhook_secret: Secret::new(&webhook_secret),
        base_url,
        live,
        directpay_unit,
        disburse_unit,
        disburse_source_account,
        payout_recipient_email,
        connect_timeout: mono::CONNECT_TIMEOUT,
        timeout: mono::REQUEST_TIMEOUT,
        insecure_http_for_tests: false,
    }))
}

/// Shipbubble's settings, read only when `SHIPBUBBLE_API_KEY` is set. Live mode
/// is `APP_ENV=production`. Startup refuses to run when:
/// - `SHIPBUBBLE_CONTACT_EMAIL` is missing or has no `@`;
/// - `SHIPBUBBLE_BASE_URL` is not https;
/// - `SHIPBUBBLE_AMOUNT_UNIT` is set to anything but `naira` / `kobo`;
/// - in live mode: the key does not start with `sb_prod`, or
///   `SHIPBUBBLE_AMOUNT_UNIT` is missing (it defaults to `naira` elsewhere);
/// - outside live mode: the key starts with `sb_prod`.
///
/// Errors name the variable, never its value.
fn shipbubble_config(
    var: &impl Fn(&str) -> Option<String>,
    live: bool,
) -> Result<Option<ShipbubbleConfig>> {
    let get = |name: &str| {
        var(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let Some(api_key) = get("SHIPBUBBLE_API_KEY") else {
        return Ok(None);
    };
    if live && !api_key.starts_with("sb_prod") {
        bail!("SHIPBUBBLE_API_KEY must be a live key (sb_prod) when APP_ENV is production");
    }
    if !live && api_key.starts_with("sb_prod") {
        bail!("SHIPBUBBLE_API_KEY is a live key but APP_ENV is not production");
    }
    let contact_email = get("SHIPBUBBLE_CONTACT_EMAIL")
        .context("SHIPBUBBLE_CONTACT_EMAIL is required when SHIPBUBBLE_API_KEY is set")?;
    if !contact_email.contains('@') || contact_email.contains(char::is_whitespace) {
        bail!("SHIPBUBBLE_CONTACT_EMAIL must be an email address");
    }
    let base_url =
        get("SHIPBUBBLE_BASE_URL").unwrap_or_else(|| shipbubble::DEFAULT_BASE_URL.into());
    if !base_url.starts_with("https://") || base_url.contains(char::is_whitespace) {
        bail!("SHIPBUBBLE_BASE_URL must be an https URL");
    }
    let amount_unit = match get("SHIPBUBBLE_AMOUNT_UNIT") {
        Some(raw) => {
            AmountUnit::parse(&raw).context("SHIPBUBBLE_AMOUNT_UNIT must be naira or kobo")?
        }
        None if live => bail!("SHIPBUBBLE_AMOUNT_UNIT is required in production (naira or kobo)"),
        None => AmountUnit::Naira,
    };
    Ok(Some(ShipbubbleConfig {
        api_key: Secret::new(&api_key),
        base_url,
        live,
        amount_unit,
        contact_email,
        connect_timeout: shipbubble::CONNECT_TIMEOUT,
        timeout: shipbubble::REQUEST_TIMEOUT,
        insecure_http_for_tests: false,
    }))
}

fn claude_config(var: &impl Fn(&str) -> Option<String>) -> Result<Option<ClaudeConfig>> {
    let get = |name: &str| {
        var(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let Some(api_key) = get("ANTHROPIC_API_KEY") else {
        return Ok(None);
    };
    let base_url = get("ANTHROPIC_BASE_URL").unwrap_or_else(|| claude::DEFAULT_BASE_URL.into());
    if !base_url.starts_with("https://") || base_url.contains(char::is_whitespace) {
        bail!("ANTHROPIC_BASE_URL must be an https URL");
    }
    Ok(Some(ClaudeConfig {
        api_key: Secret::new(&api_key),
        base_url,
        model: get("MODERATION_MODEL").unwrap_or_else(|| claude::DEFAULT_MODEL.into()),
        connect_timeout: claude::CONNECT_TIMEOUT,
        timeout: claude::REQUEST_TIMEOUT,
        insecure_http_for_tests: false,
    }))
}

/// FCM push: the service-account file (a secret kept outside the repository)
/// and an optional base URL override, which must be https.
fn fcm_config(var: &impl Fn(&str) -> Option<String>) -> Result<Option<FcmConfig>> {
    let get = |name: &str| {
        var(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let Some(file) = get("FCM_SERVICE_ACCOUNT_FILE") else {
        return Ok(None);
    };
    let mut cfg = FcmConfig::from_service_account_file(&file)?;
    if let Some(base_url) = get("FCM_BASE_URL") {
        if !base_url.starts_with("https://") || base_url.contains(char::is_whitespace) {
            bail!("FCM_BASE_URL must be an https URL");
        }
        cfg.base_url = base_url;
    }
    Ok(Some(cfg))
}

/// Which SMS provider sends codes (`SMS_PROVIDER`), and its settings. Unset means none:
/// production then refuses to serve, elsewhere codes go to the log. A name that is not a
/// provider this server has an adapter for is refused (and not echoed back).
fn sms_config(
    var: &impl Fn(&str) -> Option<String>,
) -> Result<(Option<String>, Option<TermiiConfig>)> {
    let get = |name: &str| {
        var(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let Some(provider) = get("SMS_PROVIDER").map(|v| v.to_lowercase()) else {
        return Ok((None, None));
    };
    match provider.as_str() {
        "termii" => {
            let api_key = get("TERMII_API_KEY")
                .context("TERMII_API_KEY is required when SMS_PROVIDER is termii")?;
            let sender_id = get("TERMII_SENDER_ID")
                .context("TERMII_SENDER_ID is required when SMS_PROVIDER is termii")?;
            // Alphanumeric sender IDs are 3 to 11 characters.
            if !(3..=11).contains(&sender_id.chars().count())
                || !sender_id.chars().all(|c| c.is_ascii_alphanumeric())
            {
                bail!("TERMII_SENDER_ID must be 3 to 11 letters and digits");
            }
            let base_url =
                get("TERMII_BASE_URL").unwrap_or_else(|| sms_termii::DEFAULT_BASE_URL.into());
            if !base_url.starts_with("https://") || base_url.contains(char::is_whitespace) {
                bail!("TERMII_BASE_URL must be an https URL");
            }
            Ok((
                Some(provider),
                Some(TermiiConfig {
                    api_key: Secret::new(&api_key),
                    sender_id,
                    base_url,
                    connect_timeout: sms_termii::CONNECT_TIMEOUT,
                    timeout: sms_termii::REQUEST_TIMEOUT,
                    insecure_http_for_tests: false,
                }),
            ))
        }
        _ => bail!("SMS_PROVIDER must be one of: termii"),
    }
}

/// True for `http://localhost`, `http://127.0.0.1` and `http://[::1]` (optionally with a port
/// and path): the only plain-http addresses a developer's own machine can serve.
fn is_local_http(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if authority.starts_with('[') {
        authority.split_once(']').map_or("", |(h, _)| h).to_string() + "]"
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

/// The object store's settings from the environment alone, for the `art` commands, which
/// need no database and none of the rest of the server's configuration.
pub fn s3_from_env() -> Result<Option<S3Config>> {
    let app_env = std::env::var("APP_ENV").unwrap_or_default();
    s3_config(
        &|name| std::env::var(name).ok(),
        DEV_ECHO_ENVS.contains(&app_env.as_str()),
    )
}

/// Object storage for photos. All seven variables are set together or none is.
/// `dev` (APP_ENV development or test) additionally allows a MinIO on this machine over
/// plain http; every other address, and every other environment, needs https.
fn s3_config(var: &impl Fn(&str) -> Option<String>, dev: bool) -> Result<Option<S3Config>> {
    const NAMES: [&str; 7] = [
        "S3_ENDPOINT",
        "S3_REGION",
        "S3_BUCKET_PUBLIC",
        "S3_BUCKET_PRIVATE",
        "S3_ACCESS_KEY_ID",
        "S3_SECRET_ACCESS_KEY",
        "UPLOADS_PUBLIC_BASE_URL",
    ];
    let get = |name: &str| {
        var(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let values: Vec<Option<String>> = NAMES.iter().map(|n| get(n)).collect();
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    if let Some((name, _)) = NAMES.iter().zip(&values).find(|(_, v)| v.is_none()) {
        bail!(
            "{name} is required when photo storage is configured (all of the S3_* variables and UPLOADS_PUBLIC_BASE_URL go together)"
        );
    }
    let [endpoint, region, public, private, key_id, secret, base]: [String; 7] = values
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| anyhow::anyhow!("photo storage settings are incomplete"))?;
    for (name, url) in [
        ("S3_ENDPOINT", &endpoint),
        ("UPLOADS_PUBLIC_BASE_URL", &base),
    ] {
        let allowed = url.starts_with("https://") || (dev && is_local_http(url));
        if !allowed || url.contains(char::is_whitespace) {
            bail!("{name} must be an https URL");
        }
    }
    // Only a local MinIO in development gets here with plain http.
    let plain_http = endpoint.starts_with("http://");
    let path_style = !matches!(get("S3_PATH_STYLE").as_deref(), Some("false" | "0"));
    Ok(Some(S3Config {
        endpoint,
        region,
        bucket_public: public,
        bucket_private: private,
        access_key_id: key_id,
        secret_access_key: Secret::new(&secret),
        public_base_url: base,
        path_style,
        connect_timeout: s3::CONNECT_TIMEOUT,
        timeout: s3::REQUEST_TIMEOUT,
        insecure_http_for_tests: plain_http,
    }))
}

/// Identity providers this build knows. Only the fake exists until the owner picks one.
const KNOWN_IDENTITY_PROVIDERS: &[&str] = &["fake"];

/// `IDENTITY_PROVIDER` and `NIN_INDEX_KEY` (production needing both is `main`'s check, like
/// the other providers). A provider needs the key; the key must differ
/// from the other two; the fake is refused in production; an unknown name is an error.
fn identity_config(
    var: &impl Fn(&str) -> Option<String>,
    production: bool,
    other_keys: &[[u8; 32]],
) -> Result<(Option<String>, Option<[u8; 32]>)> {
    let get = |name: &str| {
        var(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let key = get("NIN_INDEX_KEY")
        .map(|raw| key_from_b64("NIN_INDEX_KEY", &raw))
        .transpose()?;
    if key.as_ref().is_some_and(|k| other_keys.contains(k)) {
        bail!("NIN_INDEX_KEY must differ from FIELD_KEY and INDEX_KEY");
    }
    let provider = get("IDENTITY_PROVIDER").map(|v| v.to_lowercase());
    if let Some(name) = &provider {
        if !KNOWN_IDENTITY_PROVIDERS.contains(&name.as_str()) {
            bail!(
                "IDENTITY_PROVIDER is not a known provider (known: {})",
                KNOWN_IDENTITY_PROVIDERS.join(", ")
            );
        }
        if name == "fake" && production {
            bail!("IDENTITY_PROVIDER=fake is not allowed when APP_ENV is production");
        }
        if key.is_none() {
            bail!("NIN_INDEX_KEY is required when IDENTITY_PROVIDER is set");
        }
    }
    Ok((provider, key))
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Builds the config from any variable source (the process environment in production).
    pub fn from_lookup(var: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let app_env = var("APP_ENV")
            .unwrap_or_else(|| "development".into())
            .trim()
            .to_lowercase();
        let otp_dev_echo = var("OTP_DEV_ECHO").is_some_and(|v| v.trim() == "1");
        if otp_dev_echo && !DEV_ECHO_ENVS.contains(&app_env.as_str()) {
            bail!("OTP_DEV_ECHO=1 is only allowed when APP_ENV is development or test");
        }
        let key = |name: &str| {
            key_from_b64(
                name,
                &var(name).with_context(|| format!("{name} is required"))?,
            )
        };
        let field_key = key("FIELD_KEY")?;
        let index_key = key("INDEX_KEY")?;
        if field_key == index_key {
            bail!("FIELD_KEY and INDEX_KEY must differ");
        }
        let public_redirect_url = redirect_url(
            var("PUBLIC_REDIRECT_URL"),
            DEV_ECHO_ENVS.contains(&app_env.as_str()),
        )?;
        let mono = mono_config(&var, app_env == "production")?;
        let paystack = paystack_config(&var, app_env == "production")?;
        let shipbubble = shipbubble_config(&var, app_env == "production")?;
        let claude = claude_config(&var)?;
        let fcm = fcm_config(&var)?;
        let s3 = s3_config(&var, DEV_ECHO_ENVS.contains(&app_env.as_str()))?;
        let (sms_provider, termii) = sms_config(&var)?;
        let (identity_provider, nin_index_key) =
            identity_config(&var, app_env == "production", &[field_key, index_key])?;
        Ok(Self {
            identity_provider,
            nin_index_key,
            public_redirect_url,
            mono,
            paystack,
            shipbubble,
            claude,
            fcm,
            s3,
            partner: crate::partners::config::from_lookup(&var)?,
            database_url: var("DATABASE_URL").context("DATABASE_URL is required")?,
            bind_addr: var("BIND_ADDR").unwrap_or_else(|| "127.0.0.1:8080".into()),
            app_env,
            field_key,
            index_key,
            otp_dev_echo,
            app: crate::app_config::AppClientConfig::from_lookup(&var)?,
            sms_provider,
            termii,
            market_layout_dir: var("MARKET_LAYOUT_DIR")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "market".into())
                .into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const KEY_A: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc="; // 32 x 0x07
    const KEY_B: &str = "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk="; // 32 x 0x09

    fn load(pairs: &[(&str, &str)]) -> Result<Config> {
        let mut env: HashMap<&str, &str> = [
            ("DATABASE_URL", "postgres://x"),
            ("FIELD_KEY", KEY_A),
            ("INDEX_KEY", KEY_B),
            (
                "PUBLIC_REDIRECT_URL",
                "https://shop.example/checkout/return",
            ),
        ]
        .into();
        env.extend(pairs.iter().copied());
        Config::from_lookup(|k| env.get(k).map(|v| v.to_string()))
    }

    #[test]
    fn identity_config_rules() {
        const KEY_C: &str = "CwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCws=";
        // Nothing set: fine (the check is then unavailable).
        let cfg = load(&[]).unwrap();
        assert!(cfg.identity_provider.is_none() && cfg.nin_index_key.is_none());
        // The fake with its own key, outside production.
        let cfg = load(&[("IDENTITY_PROVIDER", " Fake "), ("NIN_INDEX_KEY", KEY_C)]).unwrap();
        assert_eq!(cfg.identity_provider.as_deref(), Some("fake"));
        assert_eq!(cfg.nin_index_key, Some([11u8; 32]));
        // A provider without the key, and the key equal to either other key.
        let err = load(&[("IDENTITY_PROVIDER", "fake")]).unwrap_err();
        assert!(err.to_string().contains("NIN_INDEX_KEY"), "{err}");
        for same in [KEY_A, KEY_B] {
            let err = load(&[("NIN_INDEX_KEY", same)]).unwrap_err();
            assert!(err.to_string().contains("must differ"), "{err}");
        }
        // The fake never runs in production; unknown names and bad keys are errors.
        let env = |k: &str| match k {
            "IDENTITY_PROVIDER" => Some("fake".to_string()),
            "NIN_INDEX_KEY" => Some(KEY_C.to_string()),
            _ => None,
        };
        let err = identity_config(&env, true, &[]).unwrap_err();
        assert!(err.to_string().contains("IDENTITY_PROVIDER"), "{err}");
        let err = load(&[("IDENTITY_PROVIDER", "smileid"), ("NIN_INDEX_KEY", KEY_C)]).unwrap_err();
        assert!(err.to_string().contains("not a known provider"), "{err}");
        let err = load(&[("NIN_INDEX_KEY", "short")]).unwrap_err();
        assert!(err.to_string().contains("NIN_INDEX_KEY"), "{err}");
        // The key never shows in Debug.
        let dbg = format!("{:?}", load(&[("NIN_INDEX_KEY", KEY_C)]).unwrap());
        assert!(!dbg.to_lowercase().contains("nin_index"), "{dbg}");
    }

    #[test]
    fn dev_echo_allowed_only_in_development_or_test() {
        for env in ["development", "test", " Development ", "TEST"] {
            let cfg = load(&[("APP_ENV", env), ("OTP_DEV_ECHO", "1")]).expect(env);
            assert!(cfg.otp_dev_echo);
        }
        for env in ["production", "Production ", "staging", "prod", ""] {
            let err = load(&[("APP_ENV", env), ("OTP_DEV_ECHO", "1")]).unwrap_err();
            assert!(err.to_string().contains("OTP_DEV_ECHO"), "{env}: {err}");
        }
        // Echo off is fine anywhere.
        assert!(!load(&[("APP_ENV", "production")]).unwrap().otp_dev_echo);
    }

    /// A service-account file in a fresh temp directory; returns its path.
    fn service_account(token_uri: &str) -> String {
        let dir = std::env::temp_dir().join(format!("fcm-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("service-account.json");
        let body = serde_json::json!({
            "project_id": "demo-project",
            "client_email": "push@demo-project.iam.gserviceaccount.com",
            "private_key": "placeholder",
            "token_uri": token_uri,
        });
        std::fs::write(&file, body.to_string()).unwrap();
        file.to_str().unwrap().to_string()
    }

    const S3_VARS: [(&str, &str); 7] = [
        ("S3_ENDPOINT", "https://s3.example.test"),
        ("S3_REGION", "af-south-1"),
        ("S3_BUCKET_PUBLIC", "market-public"),
        ("S3_BUCKET_PRIVATE", "market-private"),
        ("S3_ACCESS_KEY_ID", "AKEXAMPLE"),
        ("S3_SECRET_ACCESS_KEY", "never-logged-secret"),
        ("UPLOADS_PUBLIC_BASE_URL", "https://cdn.example.test"),
    ];

    const TERMII_VARS: [(&str, &str); 3] = [
        ("SMS_PROVIDER", "termii"),
        ("TERMII_API_KEY", "TLkey-never-logged-0123"),
        ("TERMII_SENDER_ID", "StockApp"),
    ];

    #[test]
    fn the_sms_provider_is_optional_and_termii_needs_its_settings() {
        let none = load(&[]).unwrap();
        assert!(none.sms_provider.is_none() && none.termii.is_none());

        let cfg = load(&TERMII_VARS).unwrap();
        assert_eq!(cfg.sms_provider.as_deref(), Some("termii"));
        let termii = cfg.termii.unwrap();
        assert_eq!(termii.sender_id, "StockApp");
        assert_eq!(termii.base_url, "https://api.ng.termii.com");
        assert!(!termii.insecure_http_for_tests);
        // Case does not matter for the provider name.
        assert!(load(&[("SMS_PROVIDER", " Termii "), TERMII_VARS[1], TERMII_VARS[2]]).is_ok());

        // Leaving out a setting names it.
        for skip in [1, 2] {
            let partial: Vec<(&str, &str)> = TERMII_VARS
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, p)| *p)
                .collect();
            let err = load(&partial).unwrap_err();
            assert!(err.to_string().contains(TERMII_VARS[skip].0), "{err}");
        }
        // The key never appears in the printed config.
        assert!(!format!("{:?}", load(&TERMII_VARS).unwrap()).contains("TLkey-never-logged"));
    }

    #[test]
    fn a_bad_sms_provider_or_sender_id_refuses_to_start() {
        let err = load(&[("SMS_PROVIDER", "not-a-provider")]).unwrap_err();
        assert!(err.to_string().contains("SMS_PROVIDER"), "{err}");
        assert!(
            !err.to_string().contains("not-a-provider"),
            "the value is not echoed: {err}"
        );

        for bad in ["ab", "TooLongSender1", "Stock App", "Stock-App", "Stöck"] {
            let mut vars = TERMII_VARS.to_vec();
            vars.push(("TERMII_SENDER_ID", bad));
            let err = load(&vars).unwrap_err();
            assert!(err.to_string().contains("TERMII_SENDER_ID"), "{bad}: {err}");
        }
        for bad in [
            "http://api.ng.termii.com",
            "api.ng.termii.com",
            "https://a b",
        ] {
            let mut vars = TERMII_VARS.to_vec();
            vars.push(("TERMII_BASE_URL", bad));
            let err = load(&vars).unwrap_err();
            assert!(err.to_string().contains("TERMII_BASE_URL"), "{bad}: {err}");
        }
        let mut vars = TERMII_VARS.to_vec();
        vars.push(("TERMII_BASE_URL", "https://sandbox.termii.test"));
        assert_eq!(
            load(&vars).unwrap().termii.unwrap().base_url,
            "https://sandbox.termii.test"
        );
    }

    #[test]
    fn a_local_minio_over_http_is_allowed_only_in_development() {
        let local = [
            ("S3_ENDPOINT", "http://localhost:9000"),
            (
                "UPLOADS_PUBLIC_BASE_URL",
                "http://127.0.0.1:9000/market-public",
            ),
        ];
        let with = |env: &'static str| -> Vec<(&str, &str)> {
            let mut v: Vec<(&str, &str)> = S3_VARS
                .iter()
                .filter(|(n, _)| !local.iter().any(|(l, _)| l == n))
                .copied()
                .collect();
            v.extend(local);
            v.push(("APP_ENV", env));
            v
        };
        for env in ["development", "test"] {
            let cfg = load(&with(env)).expect(env).s3.unwrap();
            assert!(
                cfg.insecure_http_for_tests,
                "{env}: plain http is on for the local store"
            );
        }
        for env in ["production", "staging"] {
            let err = load(&with(env)).unwrap_err();
            assert!(err.to_string().contains("https"), "{env}: {err}");
        }
        // Never another host, even in development.
        for bad in [
            "http://s3.example.test",
            "http://localhost.evil.test",
            "http://127.0.0.1.evil.test",
            "http://user@evil.test:9000",
        ] {
            let mut vars = with("development");
            vars.push(("S3_ENDPOINT", bad));
            assert!(load(&vars).is_err(), "{bad}");
        }
        // An https store in development stays strict.
        assert!(!load(&S3_VARS).unwrap().s3.unwrap().insecure_http_for_tests);
    }

    #[test]
    fn photo_storage_is_optional_and_all_or_nothing() {
        assert!(load(&[]).unwrap().s3.is_none());
        let cfg = load(&S3_VARS).unwrap().s3.unwrap();
        assert_eq!(cfg.bucket_private, "market-private");
        assert!(cfg.path_style, "path style is the default");
        assert!(!cfg.insecure_http_for_tests);
        // Leaving any one out names it.
        for (skip, (name, _)) in S3_VARS.iter().enumerate() {
            let partial: Vec<(&str, &str)> = S3_VARS
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, p)| *p)
                .collect();
            let err = load(&partial).unwrap_err();
            assert!(err.to_string().contains(name), "{err}");
        }
        let virtual_host = [&S3_VARS[..], &[("S3_PATH_STYLE", "false")]].concat();
        assert!(!load(&virtual_host).unwrap().s3.unwrap().path_style);
    }

    #[test]
    fn photo_storage_urls_must_be_https() {
        for (name, bad) in [
            ("S3_ENDPOINT", "http://s3.example.test"),
            ("UPLOADS_PUBLIC_BASE_URL", "http://cdn.example.test"),
        ] {
            let vars: Vec<(&str, &str)> = S3_VARS
                .iter()
                .map(|(k, v)| if *k == name { (*k, bad) } else { (*k, *v) })
                .collect();
            let err = load(&vars).unwrap_err();
            assert!(err.to_string().contains(name), "{err}");
        }
    }

    #[test]
    fn photo_storage_secret_is_not_in_debug() {
        let cfg = load(&S3_VARS).unwrap();
        assert!(!format!("{cfg:?}").contains("never-logged-secret"));
    }

    #[test]
    fn fcm_is_optional_and_read_from_the_file() {
        assert!(load(&[]).unwrap().fcm.is_none());
        let file = service_account("https://oauth2.googleapis.com/token");
        let cfg = load(&[("FCM_SERVICE_ACCOUNT_FILE", &file)])
            .unwrap()
            .fcm
            .unwrap();
        assert_eq!(cfg.project_id, "demo-project");
        assert_eq!(cfg.base_url, "https://fcm.googleapis.com");
        assert!(!cfg.insecure_http_for_tests);
        let cfg = load(&[
            ("FCM_SERVICE_ACCOUNT_FILE", &file),
            ("FCM_BASE_URL", "https://fcm.proxy.example"),
        ])
        .unwrap()
        .fcm
        .unwrap();
        assert_eq!(cfg.base_url, "https://fcm.proxy.example");
    }

    #[test]
    fn fcm_base_url_must_be_https() {
        let file = service_account("https://oauth2.googleapis.com/token");
        let err = load(&[
            ("FCM_SERVICE_ACCOUNT_FILE", &file),
            ("FCM_BASE_URL", "http://plain.example"),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("FCM_BASE_URL"), "{err}");
    }

    #[test]
    fn fcm_token_uri_must_be_google() {
        let file = service_account("https://evil.example/token");
        let err = load(&[("FCM_SERVICE_ACCOUNT_FILE", &file)]).unwrap_err();
        assert!(format!("{err:#}").contains("token_uri"), "{err:#}");
    }

    #[test]
    fn anthropic_base_url_must_be_https() {
        let err = load(&[
            ("ANTHROPIC_API_KEY", "sk-ant-x"),
            ("ANTHROPIC_BASE_URL", "http://plain.example"),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("ANTHROPIC_BASE_URL"), "{err}");
    }

    #[test]
    fn model_defaults_to_haiku() {
        assert!(load(&[]).unwrap().claude.is_none());
        let cfg = load(&[("ANTHROPIC_API_KEY", "sk-ant-x")])
            .unwrap()
            .claude
            .unwrap();
        assert_eq!(cfg.model, "claude-haiku-5-5");
        assert_eq!(cfg.base_url, "https://api.anthropic.com");
        let cfg = load(&[("ANTHROPIC_API_KEY", "k"), ("MODERATION_MODEL", "other")])
            .unwrap()
            .claude
            .unwrap();
        assert_eq!(cfg.model, "other");
    }

    #[test]
    fn app_env_is_trimmed_and_lowercased() {
        assert_eq!(
            load(&[("APP_ENV", " Production ")]).unwrap().app_env,
            "production"
        );
        assert_eq!(load(&[]).unwrap().app_env, "development");
    }

    #[test]
    fn redirect_url_defaults_only_in_development() {
        let dev = load(&[("PUBLIC_REDIRECT_URL", "")]).unwrap();
        assert_eq!(dev.public_redirect_url, DEV_REDIRECT_URL);
        let err = load(&[("APP_ENV", "production"), ("PUBLIC_REDIRECT_URL", "")]).unwrap_err();
        assert!(err.to_string().contains("PUBLIC_REDIRECT_URL"), "{err}");
        // Production needs https and no query string.
        for bad in [
            "http://shop.example/r",
            "https://shop.example/r?x=1",
            "ftp://x",
        ] {
            assert!(
                load(&[("APP_ENV", "production"), ("PUBLIC_REDIRECT_URL", bad)]).is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            load(&[("APP_ENV", "production")])
                .unwrap()
                .public_redirect_url,
            "https://shop.example/checkout/return"
        );
    }

    const MONO_LIVE: &[(&str, &str)] = &[
        ("APP_ENV", "production"),
        ("MONO_SECRET_KEY", "live_sk_abcdef123456"),
        ("MONO_WEBHOOK_SECRET", "whsec_abcdef"),
        ("MONO_DISBURSE_AMOUNT_UNIT", "kobo"),
        ("MONO_DISBURSE_SOURCE_ACCOUNT", "688a0b672f88771f77d05cf5"),
        ("MONO_PAYOUT_RECIPIENT_EMAIL", "payouts@platform.example"),
    ];

    fn without(
        pairs: &[(&'static str, &'static str)],
        drop: &str,
    ) -> Vec<(&'static str, &'static str)> {
        pairs.iter().copied().filter(|(k, _)| *k != drop).collect()
    }

    const PAYSTACK_LIVE: &[(&str, &str)] = &[
        ("APP_ENV", "production"),
        ("PAYSTACK_SECRET_KEY", "sk_live_abcdef123456"),
        ("PAYSTACK_PAYER_EMAIL_DOMAIN", "pay.example.ng"),
    ];

    #[test]
    fn paystack_is_optional_and_other_settings_alone_do_nothing() {
        assert!(load(&[]).unwrap().paystack.is_none());
        let cfg = load(&[
            ("APP_ENV", "production"),
            ("PAYSTACK_PAYER_EMAIL_DOMAIN", "bogus"),
            ("PAYSTACK_BASE_URL", "http://nope"),
        ])
        .unwrap();
        assert!(cfg.paystack.is_none());
    }

    #[test]
    fn paystack_live_loads_and_its_key_never_shows() {
        let p = load(PAYSTACK_LIVE).unwrap().paystack.unwrap();
        assert!(p.live);
        assert_eq!(p.base_url, "https://api.paystack.co");
        assert_eq!(p.payer_email_domain, "pay.example.ng");
        assert!(!p.insecure_http_for_tests);
        let shown = format!("{p:?}");
        assert!(!shown.contains("sk_live_abcdef123456"), "{shown}");
        let all = format!("{:?}", load(PAYSTACK_LIVE).unwrap());
        assert!(!all.contains("sk_live_abcdef123456"), "{all}");
    }

    #[test]
    fn paystack_key_prefix_must_match_the_environment() {
        // A test key in production, and a live key anywhere else, are both refused.
        let mut v = PAYSTACK_LIVE.to_vec();
        v[1].1 = "sk_test_abcdef123456";
        let err = load(&v).unwrap_err().to_string();
        assert!(
            err.contains("PAYSTACK_SECRET_KEY") && err.contains("live"),
            "{err}"
        );
        assert!(
            !err.contains("sk_test_abcdef"),
            "the value must not be echoed: {err}"
        );
        let err = load(&[("PAYSTACK_SECRET_KEY", "sk_live_abcdef123456")])
            .unwrap_err()
            .to_string();
        assert!(err.contains("PAYSTACK_SECRET_KEY"), "{err}");
        // A public key, or junk, is not a secret key.
        for bad in ["pk_test_abcdef", "not a key", "abc"] {
            assert!(load(&[("PAYSTACK_SECRET_KEY", bad)]).is_err(), "{bad}");
        }
    }

    #[test]
    fn paystack_sandbox_needs_only_a_key_and_the_other_rules_hold() {
        let p = load(&[("PAYSTACK_SECRET_KEY", "sk_test_abcdef123456")])
            .unwrap()
            .paystack
            .unwrap();
        assert!(!p.live);
        assert_eq!(p.payer_email_domain, "payer.invalid");
        // Production needs the payer email domain.
        let err = load(&without(PAYSTACK_LIVE, "PAYSTACK_PAYER_EMAIL_DOMAIN"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("PAYSTACK_PAYER_EMAIL_DOMAIN"), "{err}");
        for bad in ["nodots", "has space.ng", "-x.ng", "a@b.ng", "UPPER..ng."] {
            let mut v = PAYSTACK_LIVE.to_vec();
            v[2].1 = bad;
            assert!(load(&v).is_err(), "{bad}");
        }
        let err = load(&[
            ("PAYSTACK_SECRET_KEY", "sk_test_abcdef123456"),
            ("PAYSTACK_BASE_URL", "http://api.paystack.co"),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("PAYSTACK_BASE_URL"), "{err}");
    }

    #[test]
    fn mono_is_optional_without_a_secret_key() {
        assert!(load(&[]).unwrap().mono.is_none());
        // Other Mono settings alone neither switch it on nor demand anything.
        let cfg = load(&[
            ("APP_ENV", "production"),
            ("MONO_DIRECTPAY_AMOUNT_UNIT", "bogus"),
        ])
        .unwrap();
        assert!(cfg.mono.is_none());
    }

    #[test]
    fn mono_live_config_loads_with_defaults() {
        let mono = load(MONO_LIVE).unwrap().mono.unwrap();
        assert!(mono.live);
        assert_eq!(mono.base_url, "https://api.withmono.com");
        assert_eq!(mono.directpay_unit, AmountUnit::Kobo);
        assert_eq!(mono.disburse_unit, Some(AmountUnit::Kobo));
        assert!(!mono.insecure_http_for_tests);
    }

    #[test]
    fn mono_live_requires_disburse_settings() {
        for name in [
            "MONO_WEBHOOK_SECRET",
            "MONO_DISBURSE_AMOUNT_UNIT",
            "MONO_DISBURSE_SOURCE_ACCOUNT",
            "MONO_PAYOUT_RECIPIENT_EMAIL",
        ] {
            let err = load(&without(MONO_LIVE, name)).unwrap_err();
            assert!(err.to_string().contains(name), "{name}: {err}");
        }
    }

    #[test]
    fn mono_key_prefix_must_match_the_environment() {
        let mut test_key_in_prod = without(MONO_LIVE, "MONO_SECRET_KEY");
        test_key_in_prod.push(("MONO_SECRET_KEY", "test_sk_abcdef"));
        let err = load(&test_key_in_prod).unwrap_err();
        assert!(err.to_string().contains("live key"), "{err}");
        // Anything that does not say live is refused in production.
        for key in ["sk_abcdef", "LIVE_SK_abc", "abc_live_sk_"] {
            let mut env = without(MONO_LIVE, "MONO_SECRET_KEY");
            env.push(("MONO_SECRET_KEY", key));
            let err = load(&env).unwrap_err().to_string();
            assert!(err.contains("live key"), "{key}: {err}");
            assert!(!err.contains(key), "{key}: value echoed");
        }
        let err = load(&[
            ("MONO_SECRET_KEY", "live_sk_abcdef"),
            ("MONO_WEBHOOK_SECRET", "whsec"),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("live key"), "{err}");
    }

    #[test]
    fn mono_sandbox_needs_only_key_and_webhook_secret() {
        let mono = load(&[
            ("MONO_SECRET_KEY", "test_sk_abcdef"),
            ("MONO_WEBHOOK_SECRET", "whsec"),
        ])
        .unwrap()
        .mono
        .unwrap();
        assert!(!mono.live);
        assert_eq!(mono.disburse_unit, None);
        assert_eq!(mono.disburse_source_account, None);
    }

    #[test]
    fn mono_invalid_values_refused() {
        for (name, value) in [
            ("MONO_DIRECTPAY_AMOUNT_UNIT", "ngn"),
            ("MONO_DISBURSE_AMOUNT_UNIT", "cents"),
            ("MONO_BASE_URL", "http://api.withmono.com"),
            ("MONO_PAYOUT_RECIPIENT_EMAIL", "not-an-email"),
        ] {
            let mut env = without(MONO_LIVE, name);
            env.push((name, value));
            let err = load(&env).unwrap_err().to_string();
            assert!(err.contains(name), "{name}: {err}");
            assert!(!err.contains(value), "{name}: value echoed");
        }
        let mut naira = without(MONO_LIVE, "MONO_DIRECTPAY_AMOUNT_UNIT");
        naira.push(("MONO_DIRECTPAY_AMOUNT_UNIT", "Naira"));
        assert_eq!(
            load(&naira).unwrap().mono.unwrap().directpay_unit,
            AmountUnit::Naira
        );
    }

    #[test]
    fn config_debug_hides_mono_secrets() {
        let cfg = load(MONO_LIVE).unwrap();
        let out = format!("{cfg:?} {cfg:#?}");
        assert!(!out.contains("live_sk_abcdef123456"), "{out}");
        assert!(!out.contains("whsec_abcdef"), "{out}");
    }

    const SB_LIVE: &[(&str, &str)] = &[
        ("APP_ENV", "production"),
        ("SHIPBUBBLE_API_KEY", "sb_prod_abcdef123456"),
        ("SHIPBUBBLE_AMOUNT_UNIT", "naira"),
        ("SHIPBUBBLE_CONTACT_EMAIL", "ops@platform.example"),
    ];

    #[test]
    fn shipbubble_is_optional_without_a_key() {
        assert!(load(&[]).unwrap().shipbubble.is_none());
        let cfg = load(&[
            ("APP_ENV", "production"),
            ("SHIPBUBBLE_AMOUNT_UNIT", "bogus"),
        ])
        .unwrap();
        assert!(cfg.shipbubble.is_none());
    }

    #[test]
    fn shipbubble_live_config_loads() {
        let sb = load(SB_LIVE).unwrap().shipbubble.unwrap();
        assert!(sb.live);
        assert_eq!(sb.base_url, "https://api.shipbubble.com/v1/shipping");
        assert_eq!(sb.amount_unit, AmountUnit::Naira);
        assert!(!sb.insecure_http_for_tests);
    }

    #[test]
    fn shipbubble_production_requires_unit_and_sb_prod_key() {
        let err = load(&without(SB_LIVE, "SHIPBUBBLE_AMOUNT_UNIT")).unwrap_err();
        assert!(err.to_string().contains("SHIPBUBBLE_AMOUNT_UNIT"), "{err}");
        for key in ["sb_sandbox_abc", "SB_PROD_abc", "abc_sb_prod"] {
            let mut env = without(SB_LIVE, "SHIPBUBBLE_API_KEY");
            env.push(("SHIPBUBBLE_API_KEY", key));
            let err = load(&env).unwrap_err().to_string();
            assert!(err.contains("sb_prod"), "{key}: {err}");
            assert!(!err.contains(key), "{key}: value echoed");
        }
        let err = load(&[("SHIPBUBBLE_API_KEY", "sb_prod_abcdef")]).unwrap_err();
        assert!(err.to_string().contains("live key"), "{err}");
    }

    #[test]
    fn shipbubble_requires_a_contact_email() {
        let err = load(&without(SB_LIVE, "SHIPBUBBLE_CONTACT_EMAIL")).unwrap_err();
        assert!(
            err.to_string().contains("SHIPBUBBLE_CONTACT_EMAIL"),
            "{err}"
        );
        let mut env = without(SB_LIVE, "SHIPBUBBLE_CONTACT_EMAIL");
        env.push(("SHIPBUBBLE_CONTACT_EMAIL", "not-an-email"));
        let err = load(&env).unwrap_err().to_string();
        assert!(err.contains("SHIPBUBBLE_CONTACT_EMAIL"), "{err}");
        assert!(!err.contains("not-an-email"), "value echoed");
        let sb = load(SB_LIVE).unwrap().shipbubble.unwrap();
        assert_eq!(sb.contact_email, "ops@platform.example");
    }

    #[test]
    fn shipbubble_sandbox_defaults_to_naira() {
        let sb = load(&[
            ("SHIPBUBBLE_API_KEY", "sb_sandbox_abcdef"),
            ("SHIPBUBBLE_CONTACT_EMAIL", "ops@platform.example"),
        ])
        .unwrap()
        .shipbubble
        .unwrap();
        assert!(!sb.live);
        assert_eq!(sb.amount_unit, AmountUnit::Naira);
    }

    #[test]
    fn shipbubble_invalid_values_refused_and_debug_hides_key() {
        for (name, value) in [
            ("SHIPBUBBLE_AMOUNT_UNIT", "cents"),
            ("SHIPBUBBLE_BASE_URL", "http://api.shipbubble.com"),
        ] {
            let mut env = without(SB_LIVE, name);
            env.push((name, value));
            let err = load(&env).unwrap_err().to_string();
            assert!(err.contains(name), "{name}: {err}");
            assert!(!err.contains(value), "{name}: value echoed");
        }
        let cfg = load(SB_LIVE).unwrap();
        let out = format!("{cfg:?} {cfg:#?}");
        assert!(!out.contains("sb_prod_abcdef123456"), "{out}");
    }

    #[test]
    fn identical_field_and_index_keys_refused() {
        let err = load(&[("INDEX_KEY", KEY_A)]).unwrap_err();
        assert!(
            err.to_string()
                .contains("FIELD_KEY and INDEX_KEY must differ"),
            "{err}"
        );
    }
}
