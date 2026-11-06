//! Language eval for the live tone check. It spends money and needs a key, so
//! it is ignored by default and is not part of the suite. Run it by hand:
//!
//! ```text
//! ANTHROPIC_API_KEY=... cargo test --test tone_eval -- --ignored --nocapture
//! ```
//!
//! It prints a confusion table per language and asserts nothing about a rate:
//! the owner sets that bar. All fixture text is invented, never real messages,
//! and it still needs review by native speakers.

use std::{collections::BTreeMap, time::Duration};

use serde::Deserialize;
use server::{
    moderation::{
        claude::{self, ClaudeConfig, ClaudeTone},
        filter::Severity,
        tone::ToneChecker,
    },
    payments::mono::Secret,
};

const FIXTURE: &str = include_str!("fixtures/tone_eval.json");
const LANGUAGES: [&str; 6] = ["english", "pidgin", "hausa", "yoruba", "igbo", "mixed"];
const EXPECTED: [&str; 3] = ["clean", "mild", "serious"];
const MIN_ROWS: usize = 12;

#[derive(Deserialize)]
struct Row {
    lang: String,
    text: String,
    expected: String,
}

fn rows() -> Vec<Row> {
    serde_json::from_str(FIXTURE).expect("the fixture is valid JSON")
}

/// Runs in the normal suite, with no key and no network.
#[test]
fn fixture_covers_every_language_and_outcome() {
    let rows = rows();
    for lang in LANGUAGES {
        let in_lang: Vec<&Row> = rows.iter().filter(|r| r.lang == lang).collect();
        assert!(in_lang.len() >= MIN_ROWS, "{lang}: {}", in_lang.len());
        for expected in EXPECTED {
            assert!(
                in_lang.iter().any(|r| r.expected == expected),
                "{lang} has no {expected} row"
            );
        }
    }
    assert!(rows.iter().all(|r| LANGUAGES.contains(&r.lang.as_str())));
    assert!(rows.iter().all(|r| EXPECTED.contains(&r.expected.as_str())));
    assert!(rows.iter().all(|r| !r.text.trim().is_empty()));
}

fn tone_from_env() -> Option<ClaudeTone> {
    let key = std::env::var("ANTHROPIC_API_KEY").ok()?;
    let model =
        std::env::var("MODERATION_MODEL").unwrap_or_else(|_| claude::DEFAULT_MODEL.to_string());
    let tone = ClaudeTone::new(ClaudeConfig {
        api_key: Secret::new(&key),
        base_url: claude::DEFAULT_BASE_URL.into(),
        model,
        connect_timeout: claude::CONNECT_TIMEOUT,
        timeout: Duration::from_secs(30),
        insecure_http_for_tests: false,
    })
    .expect("build the client");
    Some(tone)
}

#[tokio::test]
#[ignore = "spends money: needs ANTHROPIC_API_KEY"]
async fn language_confusion_table() {
    let Some(tone) = tone_from_env() else {
        println!("ANTHROPIC_API_KEY is not set, so nothing ran");
        return;
    };
    // (language, expected, actual) -> count
    let mut table: BTreeMap<(String, String, &str), u32> = BTreeMap::new();
    for row in rows() {
        let actual = match tone.check(&row.text).await {
            Ok(verdict) => match verdict.severity {
                None => "clean",
                Some(Severity::Mild) => "mild",
                Some(Severity::Serious) => "serious",
            },
            Err(_) => "failed",
        };
        *table.entry((row.lang, row.expected, actual)).or_default() += 1;
    }
    for lang in LANGUAGES {
        println!(
            "\n{lang}\n{:<10}{:>8}{:>8}{:>9}{:>8}",
            "expected", "clean", "mild", "serious", "failed"
        );
        for expected in EXPECTED {
            let count = |actual: &str| {
                table
                    .iter()
                    .find(|((l, e, a), _)| l == lang && e == expected && *a == actual)
                    .map_or(0, |(_, n)| *n)
            };
            println!(
                "{expected:<10}{:>8}{:>8}{:>9}{:>8}",
                count("clean"),
                count("mild"),
                count("serious"),
                count("failed")
            );
        }
    }
}
