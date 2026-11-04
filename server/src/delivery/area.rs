//! The FCT service area: the six area councils and the NIPOST postcode check.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

const COMING_SOON: &str = "Coming to your city soon";

/// The six FCT area councils. Stored in Postgres as `area_council`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "area_council", rename_all = "snake_case")]
pub enum AreaCouncil {
    Amac,
    Bwari,
    Gwagwalada,
    Kuje,
    Kwali,
    Abaji,
}

impl AreaCouncil {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Amac => "amac",
            Self::Bwari => "bwari",
            Self::Gwagwalada => "gwagwalada",
            Self::Kuje => "kuje",
            Self::Kwali => "kwali",
            Self::Abaji => "abaji",
        }
    }
}

fn outside() -> AppError {
    AppError::Unprocessable("outside_service_area", COMING_SOON.into())
}

impl FromStr for AreaCouncil {
    type Err = AppError;

    /// Case-insensitive. "abuja municipal" is an alias of AMAC. Nothing else is
    /// inferred (never from the word "Abuja" alone).
    fn from_str(s: &str) -> Result<Self, AppError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "amac" | "abuja municipal" => Ok(Self::Amac),
            "bwari" => Ok(Self::Bwari),
            "gwagwalada" => Ok(Self::Gwagwalada),
            "kuje" => Ok(Self::Kuje),
            "kwali" => Ok(Self::Kwali),
            "abaji" => Ok(Self::Abaji),
            _ => Err(outside()),
        }
    }
}

/// Normalises a NIPOST postcode: strips spaces and hyphens, uppercases. Must be
/// the new 11-character format and start with `FC`.
pub fn normalize_postcode(raw: &str) -> Result<String, AppError> {
    let code: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();
    if code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AppError::Unprocessable(
            "invalid_postcode",
            "Use the new 11-character postcode, or leave it blank".into(),
        ));
    }
    let b = code.as_bytes();
    let well_formed = b.len() == 11
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..4].iter().all(u8::is_ascii_digit)
        && b[4..]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if !well_formed {
        return Err(AppError::Unprocessable(
            "invalid_postcode",
            "The postcode must be 11 characters, such as FC02A09DB09".into(),
        ));
    }
    if !code.starts_with("FC") {
        return Err(outside());
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: AppError) -> &'static str {
        match e {
            AppError::Unprocessable(c, _) => c,
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn postcode_normalises_and_requires_fc() {
        assert_eq!(normalize_postcode("fc-02 a09db09").unwrap(), "FC02A09DB09");
        assert_eq!(
            code(normalize_postcode("LA02A09DB09").unwrap_err()),
            "outside_service_area"
        );
        assert_eq!(
            code(normalize_postcode("FC02").unwrap_err()),
            "invalid_postcode"
        );
    }

    #[test]
    fn legacy_six_digit_postcode_is_invalid_with_guidance() {
        match normalize_postcode("900001").unwrap_err() {
            AppError::Unprocessable(c, m) => {
                assert_eq!(c, "invalid_postcode");
                assert_eq!(m, "Use the new 11-character postcode, or leave it blank");
            }
            other => panic!("unexpected error {other:?}"),
        }
        assert_eq!(
            code(normalize_postcode("90 00-01").unwrap_err()),
            "invalid_postcode"
        );
    }

    #[test]
    fn area_council_rejects_unknown() {
        assert_eq!(
            code("suleja".parse::<AreaCouncil>().unwrap_err()),
            "outside_service_area"
        );
        assert_eq!(
            code("abuja".parse::<AreaCouncil>().unwrap_err()),
            "outside_service_area"
        );
    }

    #[test]
    fn area_council_parses_case_insensitively_with_alias() {
        assert_eq!("AMAC".parse::<AreaCouncil>().unwrap(), AreaCouncil::Amac);
        assert_eq!(
            "Abuja Municipal".parse::<AreaCouncil>().unwrap(),
            AreaCouncil::Amac
        );
        assert_eq!(
            " Kwali ".parse::<AreaCouncil>().unwrap(),
            AreaCouncil::Kwali
        );
    }
}
