//! The shared check: offline filters first, then the tone check.

use super::{
    filter::{Category, Scan, Severity, scan, scan_split_digits},
    tone::{ToneChecker, ToneError},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagSeverity {
    Mild,
    Serious,
    Unreviewable,
}

impl FlagSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            FlagSeverity::Mild => "mild",
            FlagSeverity::Serious => "serious",
            FlagSeverity::Unreviewable => "unreviewable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Filter,
    Tone,
    System,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Filter => "filter",
            Source::Tone => "tone",
            Source::System => "system",
        }
    }
}

/// `Rewrite` carries message text, so `Debug` leaves the suggestion out.
#[derive(Clone, PartialEq, Eq)]
pub enum Verdict {
    Clean,
    Rewrite {
        category: Category,
        suggestion: Option<String>,
        source: Source,
    },
    Block {
        severity: FlagSeverity,
        category: Category,
        source: Source,
    },
}

impl std::fmt::Debug for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verdict::Clean => f.write_str("Clean"),
            Verdict::Rewrite {
                category,
                suggestion,
                ..
            } => write!(
                f,
                "Rewrite({category:?}, offered: {})",
                suggestion.is_some()
            ),
            Verdict::Block {
                severity, category, ..
            } => write!(f, "Block({severity:?}, {category:?})"),
        }
    }
}

/// Judges one text. Offline checks come first and Claude cannot overrule them:
/// a serious hit never leaves the server, and a mild word stays mild whatever
/// the model says. `recent` is the sender's latest messages, oldest first.
/// `ToneError::Unavailable` passes up: the caller holds or refuses, and never
/// treats it as clean.
pub async fn judge(
    tone: &dyn ToneChecker,
    text: &str,
    recent: &[&str],
    max_chars: usize,
) -> Result<Verdict, ToneError> {
    let offline = match scan(text) {
        Scan::Clean => scan_split_digits(recent, text),
        hit => hit,
    };
    if let Scan::Hit {
        severity: Severity::Serious,
        category,
    } = offline
    {
        return Ok(Verdict::Block {
            severity: FlagSeverity::Serious,
            category,
            source: Source::Filter,
        });
    }

    let answer = match tone.check(text).await {
        Ok(answer) => answer,
        Err(ToneError::Refused) => {
            return Ok(Verdict::Block {
                severity: FlagSeverity::Unreviewable,
                category: Category::Unreviewable,
                source: Source::System,
            });
        }
        Err(e) => return Err(e),
    };

    let leaves_app = matches!(
        answer.category,
        Some(Category::Scam | Category::OffPlatform)
    );
    if answer.severity == Some(Severity::Serious) || leaves_app {
        return Ok(Verdict::Block {
            severity: FlagSeverity::Serious,
            category: match answer.category {
                Some(c @ (Category::Scam | Category::OffPlatform)) => c,
                _ => Category::Abuse,
            },
            source: Source::Tone,
        });
    }

    let tone_mild = answer.severity == Some(Severity::Mild);
    if tone_mild || offline != Scan::Clean {
        return Ok(Verdict::Rewrite {
            category: Category::Rude,
            suggestion: answer
                .suggestion
                .filter(|s| s.chars().count() <= max_chars && scan(s) == Scan::Clean),
            source: if tone_mild {
                Source::Tone
            } else {
                Source::Filter
            },
        });
    }
    Ok(Verdict::Clean)
}

/// What the sender is told. Fixed copy, never the model's words.
pub fn reason_copy(category: Category) -> &'static str {
    match category {
        Category::OffPlatform | Category::Scam => {
            "Keep contact details and payments inside the app. This message wasn't sent."
        }
        Category::Abuse => "This message breaks the market's rules and wasn't sent.",
        Category::Unreviewable => {
            "We couldn't check this message, so it wasn't sent. Try wording it differently."
        }
        Category::Rude => "This could come across as rude. Send this version instead?",
    }
}
