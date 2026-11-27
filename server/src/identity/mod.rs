//! Seller identity check: a NIN and a live selfie, matched against NIMC by a
//! licensed provider. Everything that talks to the provider goes through
//! [`IdentityProvider`]; [`fake::FakeIdentity`] is the development and test double.
//!
//! Data protection: the NIN is a [`Nin`], held in memory for one call, never
//! written anywhere and never printed. What we keep is a keyed fingerprint
//! ([`fingerprint`], under `NIN_INDEX_KEY`, its own key), the verified name
//! (encrypted) and the year of birth. Types that hold a name or token have a
//! hand-written `Debug` that redacts it, and [`IdentityError`] never carries detail.

pub mod fake;
pub mod jobs;
pub mod routes;
pub mod service;
pub mod smile;

use std::fmt;

use async_trait::async_trait;
use axum::http::HeaderMap;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::crypto::blind_index;

/// A National Identification Number: exactly 11 ASCII digits. Anything else is
/// refused before any other work.
pub struct Nin(String);

impl Nin {
    pub fn parse(raw: &str) -> Option<Nin> {
        (raw.len() == 11 && raw.bytes().all(|b| b.is_ascii_digit())).then(|| Nin(raw.to_string()))
    }

    /// The digits, for the provider call and the fingerprint only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Nin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Nin(<redacted>)")
    }
}

/// The keyed fingerprint of a NIN: it recognises a NIN but cannot be read back.
pub fn fingerprint(key: &[u8; 32], nin: &Nin) -> Vec<u8> {
    blind_index(key, nin.expose())
}

/// What the provider verified. Personal data: `Debug` redacts it.
#[derive(Clone, PartialEq, Eq)]
pub struct Verified {
    pub full_name: String,
    pub date_of_birth: Date,
}

impl fmt::Debug for Verified {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Verified(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    NotFound,
    FaceMismatch,
    NotAlive,
}

impl FailReason {
    pub fn code(self) -> &'static str {
        match self {
            FailReason::NotFound => "not_found",
            FailReason::FaceMismatch => "face_mismatch",
            FailReason::NotAlive => "not_alive",
        }
    }
}

/// The provider's answer for one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Passed(Verified),
    Failed(FailReason),
    /// The provider could not decide: a person looks.
    Unsure,
}

#[derive(Clone)]
pub struct StartedJob {
    pub provider_ref: String,
    /// What the app's SDK needs to capture the selfie. Sent to the app, never logged.
    pub session_token: String,
}

impl fmt::Debug for StartedJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StartedJob(<redacted>)")
    }
}

/// What the app's SDK needs besides the session token. Not secret: the partner id is
/// public to the SDK by design, and the token is what authorises it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SdkParams {
    pub partner_id: String,
    pub id_type: &'static str,
    pub country: &'static str,
    pub sandbox: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityEvent {
    pub provider_ref: String,
    pub outcome: Outcome,
}

/// Carries no detail: a provider's body can hold a NIN or a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("invalid signature")]
    InvalidSignature,
    #[error("malformed provider message")]
    Malformed,
    #[error("identity provider unavailable")]
    Unavailable,
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Settings the app's SDK needs; `None` for a provider without an SDK step.
    fn sdk_params(&self) -> Option<SdkParams> {
        None
    }

    /// Opens a job for the NIN. `reference` is our attempt id, which carries no
    /// personal data and may be sent as the provider's own reference. `display_name` is
    /// the name the seller gave us, for providers that want one; it is never stored.
    async fn start(
        &self,
        nin: &Nin,
        reference: Uuid,
        display_name: &str,
    ) -> Result<StartedJob, IdentityError>;

    /// Verifies the signature over the raw bytes first, then reads the result.
    fn parse_webhook(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<IdentityEvent, IdentityError>;

    /// The job's result if the provider has one (the poll job's backup for a missing
    /// webhook); `None` while it is still running.
    async fn fetch(&self, provider_ref: &str) -> Result<Option<Outcome>, IdentityError>;
}

const TITLES: [&str; 12] = [
    "mr", "mrs", "miss", "ms", "dr", "prof", "chief", "engr", "alh", "alhaji", "alhaja", "hon",
];

/// Lower-cased words of a name: punctuation and titles dropped, an apostrophe
/// joining (`O'Brien` is `obrien`), a hyphen or other mark separating.
fn words(name: &str) -> Vec<String> {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '\'' | '\u{2019}' | '`'))
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    cleaned
        .to_lowercase()
        .split_whitespace()
        .filter(|w| !TITLES.contains(w))
        .map(str::to_string)
        .collect()
}

/// Whether two names are the same person's: every word of the shorter name is in the
/// longer one, in any order, and an initial matches a word's first letter. The shorter
/// name needs at least one full word, so initials alone never match.
pub fn names_match(a: &str, b: &str) -> bool {
    let (wa, wb) = (words(a), words(b));
    let (short, long) = if wa.len() <= wb.len() {
        (wa, wb)
    } else {
        (wb, wa)
    };
    if !short.iter().any(|w| w.chars().count() > 1) {
        return false;
    }
    let mut used = vec![false; long.len()];
    // Full words first, so an initial cannot take the word a full match needs.
    let mut order: Vec<&String> = short.iter().collect();
    order.sort_by_key(|w| w.chars().count() == 1);
    order.into_iter().all(|w| {
        let initial = w.chars().count() == 1;
        let hit = long.iter().enumerate().position(|(i, l)| {
            !used[i]
                && if initial {
                    l.starts_with(w.as_str())
                } else {
                    l == w
                }
        });
        match hit {
            Some(i) => {
                used[i] = true;
                true
            }
            None => false,
        }
    })
}

/// Whole years between `born` and `today`.
pub fn age_on(born: Date, today: Date) -> i32 {
    let mut age = today.year() - born.year();
    if (today.month() as u8, today.day()) < (born.month() as u8, born.day()) {
        age -= 1;
    }
    age
}

pub fn today() -> Date {
    OffsetDateTime::now_utc().date()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    #[test]
    fn nin_is_exactly_eleven_digits() {
        assert!(Nin::parse("12345678901").is_some());
        for bad in [
            "",
            "1234567890",
            "123456789012",
            "1234567890a",
            "12345 678901",
            "１２３４５６７８９０１",
            " 12345678901",
        ] {
            assert!(Nin::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn nin_debug_hides_the_digits() {
        let nin = Nin::parse("12345678901").unwrap();
        assert!(!format!("{nin:?}").contains("1234"));
    }

    #[test]
    fn fingerprint_is_keyed_and_stable() {
        let nin = Nin::parse("12345678901").unwrap();
        let other = Nin::parse("12345678902").unwrap();
        assert_eq!(fingerprint(&[1; 32], &nin), fingerprint(&[1; 32], &nin));
        assert_ne!(fingerprint(&[1; 32], &nin), fingerprint(&[2; 32], &nin));
        assert_ne!(fingerprint(&[1; 32], &nin), fingerprint(&[1; 32], &other));
        assert_eq!(fingerprint(&[1; 32], &nin).len(), 32);
    }

    #[test]
    fn names_match_in_any_order_with_initials() {
        assert!(names_match("Ada Obi", "OBI ADA CHINWE"));
        assert!(names_match("Mrs. Ada Obi", "ada obi"));
        assert!(names_match("Ada O'Brien", "ADA OBRIEN"));
        assert!(names_match("ada-obi", "Obi, Ada"));
        assert!(names_match("A. Obi", "Ada Chinwe Obi"));
        assert!(names_match("Ada", "ADA OBI"));
        assert!(!names_match("Ada Obi", "CHIDI OKAFOR"));
        assert!(!names_match("Ada Obi", "ADA OKAFOR"));
        // Initials alone, or nothing, never match.
        assert!(!names_match("A B", "Ada Bello"));
        assert!(!names_match("", "Ada Obi"));
        assert!(!names_match("Dr", "Dr Ada"));
        // One initial cannot cover two words.
        assert!(!names_match("A. A. Obi", "Ada Obi"));
        assert!(names_match("A. A. Obi", "Ada Amaka Obi"));
    }

    #[test]
    fn age_counts_birthdays() {
        assert_eq!(age_on(date!(2008 - 06 - 15), date!(2026 - 06 - 14)), 17);
        assert_eq!(age_on(date!(2008 - 06 - 15), date!(2026 - 06 - 15)), 18);
        assert_eq!(age_on(date!(1990 - 01 - 01), date!(2026 - 11 - 26)), 36);
    }
}
