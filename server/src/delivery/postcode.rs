//! The postcode check. Coverage of the NIPOST postcode is still incomplete, so a
//! check never blocks saving an address unless the code is definitely invalid.

use async_trait::async_trait;

/// What a postcode lookup concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostcodeCheck {
    /// The code exists.
    Valid,
    /// The code is well-formed but cannot be confirmed either way.
    Unknown,
    /// The code definitely does not exist.
    Invalid,
}

/// The lookup could not be completed. Carries no detail, so nothing leaks to logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderError {
    Unavailable,
}

#[async_trait]
pub trait PostcodeProvider: Send + Sync {
    /// `code` is already normalised (see `area::normalize_postcode`).
    async fn check(&self, code: &str) -> Result<PostcodeCheck, ProviderError>;
}

// REVISIT(nipost): a real adapter belongs here once NIPOST publishes a documented
// API and issues a key. Until then `ManualPostcode` is the only provider.

/// Confirms nothing: any well-formed code is `Unknown`, so saving is never blocked.
#[derive(Debug, Default, Clone, Copy)]
pub struct ManualPostcode;

#[async_trait]
impl PostcodeProvider for ManualPostcode {
    async fn check(&self, _code: &str) -> Result<PostcodeCheck, ProviderError> {
        Ok(PostcodeCheck::Unknown)
    }
}
