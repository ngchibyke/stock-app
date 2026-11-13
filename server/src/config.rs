use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::couriers::shipbubble::{self, ShipbubbleConfig};
use crate::moderation::claude::{self, ClaudeConfig};
use crate::notify::fcm::FcmConfig;
use crate::payments::mono::{self, AmountUnit, MonoConfig, Secret};

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
    /// Shipbubble settings; `None` when `SHIPBUBBLE_API_KEY` is unset (courier
    /// delivery is then unavailable; the server still starts).
    pub shipbubble: Option<ShipbubbleConfig>,
    /// Claude settings; `None` when `ANTHROPIC_API_KEY` is unset (production
    /// then refuses to serve).
    pub claude: Option<ClaudeConfig>,
    /// FCM settings; `None` when `FCM_SERVICE_ACCOUNT_FILE` is unset
    /// (production then refuses to serve; elsewhere pushes are dropped).
    pub fcm: Option<FcmConfig>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("app_env", &self.app_env)
            .field("mono", &self.mono)
            .field("shipbubble", &self.shipbubble)
            .field("claude", &self.claude)
            .field("fcm", &self.fcm)
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
        let shipbubble = shipbubble_config(&var, app_env == "production")?;
        let claude = claude_config(&var)?;
        let fcm = fcm_config(&var)?;
        Ok(Self {
            public_redirect_url,
            mono,
            shipbubble,
            claude,
            fcm,
            database_url: var("DATABASE_URL").context("DATABASE_URL is required")?,
            bind_addr: var("BIND_ADDR").unwrap_or_else(|| "127.0.0.1:8080".into()),
            app_env,
            field_key,
            index_key,
            otp_dev_echo,
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
