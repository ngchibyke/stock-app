//! `PARTNER_API_BASE_URL`, `PARTNER_API_KEY` and `PARTNER_WEBHOOK_SECRET`: all three or none.
//! A partner is optional, so none is fine, even in production.

use std::fmt;

use anyhow::{Result, bail};

use crate::payments::mono::Secret;

#[derive(Clone)]
pub struct PartnerConfig {
    pub base_url: String,
    pub api_key: Secret,
    pub webhook_secret: Secret,
    /// **Tests only** (a local mock speaks plain http). Config always sets `false`.
    pub insecure_http_for_tests: bool,
}

impl fmt::Debug for PartnerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PartnerConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"[redacted]")
            .field("webhook_secret", &"[redacted]")
            .finish()
    }
}

pub fn from_lookup(var: &impl Fn(&str) -> Option<String>) -> Result<Option<PartnerConfig>> {
    let get = |name: &str| {
        var(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let (url, key, secret) = (
        get("PARTNER_API_BASE_URL"),
        get("PARTNER_API_KEY"),
        get("PARTNER_WEBHOOK_SECRET"),
    );
    let (Some(url), Some(key), Some(secret)) = (url.clone(), key.clone(), secret.clone()) else {
        if url.is_some() || key.is_some() || secret.is_some() {
            bail!(
                "PARTNER_API_BASE_URL, PARTNER_API_KEY and PARTNER_WEBHOOK_SECRET must be set together or not at all"
            );
        }
        return Ok(None);
    };
    if !url.starts_with("https://") || url.contains(char::is_whitespace) || url.len() > 200 {
        bail!("PARTNER_API_BASE_URL must be an https URL");
    }
    if key.contains(char::is_control) || secret.len() < 16 {
        bail!(
            "PARTNER_API_KEY is not usable or PARTNER_WEBHOOK_SECRET is shorter than 16 characters"
        );
    }
    Ok(Some(PartnerConfig {
        base_url: url.trim_end_matches('/').to_string(),
        api_key: Secret::new(&key),
        webhook_secret: Secret::new(&secret),
        insecure_http_for_tests: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(pairs: &[(&str, &str)]) -> Result<Option<PartnerConfig>> {
        from_lookup(&|n| {
            pairs
                .iter()
                .find(|(k, _)| *k == n)
                .map(|(_, v)| v.to_string())
        })
    }

    const OK: [(&str, &str); 3] = [
        ("PARTNER_API_BASE_URL", "https://partner.example.test/"),
        ("PARTNER_API_KEY", "key-never-logged"),
        ("PARTNER_WEBHOOK_SECRET", "secret-of-16-chars-plus"),
    ];

    #[test]
    fn none_is_fine_all_three_work_and_part_is_refused() {
        assert!(load(&[]).unwrap().is_none());
        let cfg = load(&OK).unwrap().unwrap();
        assert_eq!(cfg.base_url, "https://partner.example.test");
        for skip in 0..3 {
            let part: Vec<_> = OK
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, p)| *p)
                .collect();
            assert!(load(&part).is_err(), "missing {skip}");
        }
    }

    #[test]
    fn the_address_must_be_https_and_the_secret_long_enough() {
        let mut v = OK.to_vec();
        v[0].1 = "http://partner.example.test";
        assert!(load(&v).unwrap_err().to_string().contains("https"));
        let mut v = OK.to_vec();
        v[2].1 = "short";
        assert!(load(&v).is_err());
    }

    #[test]
    fn debug_never_prints_the_key_or_the_secret() {
        let cfg = load(&OK).unwrap().unwrap();
        let text = format!("{cfg:?}");
        assert!(!text.contains("key-never-logged") && !text.contains("secret-of-16"));
    }
}
