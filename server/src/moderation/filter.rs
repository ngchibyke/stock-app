//! Offline checks: no network, no database.

use unicode_normalization::UnicodeNormalization;

use super::words::{LINK_MARKERS, MILD_WORDS, OFF_PLATFORM_PHRASES, SCAM_PHRASES, SERIOUS_WORDS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Mild,
    Serious,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    OffPlatform,
    Scam,
    Abuse,
    Rude,
    Unreviewable,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::OffPlatform => "off_platform",
            Category::Scam => "scam",
            Category::Abuse => "abuse",
            Category::Rude => "rude",
            Category::Unreviewable => "unreviewable",
        }
    }

    /// Reads back a stored `as_str` value.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "off_platform" => Category::OffPlatform,
            "scam" => Category::Scam,
            "abuse" => Category::Abuse,
            "rude" => Category::Rude,
            "unreviewable" => Category::Unreviewable,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scan {
    Clean,
    Hit {
        severity: Severity,
        category: Category,
    },
}

const OFF_PLATFORM: Scan = Scan::Hit {
    severity: Severity::Serious,
    category: Category::OffPlatform,
};
const SCAM: Scan = Scan::Hit {
    severity: Severity::Serious,
    category: Category::Scam,
};
const ABUSE: Scan = Scan::Hit {
    severity: Severity::Serious,
    category: Category::Abuse,
};
const RUDE: Scan = Scan::Hit {
    severity: Severity::Mild,
    category: Category::Rude,
};

/// Words that show a spaced ten-digit run is an account number, not prices.
const BANK_WORDS: &[&str] = &["account", "acct", "bank", "transfer"];

const NIGERIAN_PREFIXES: &[&str] = &["70", "80", "81", "90", "91"];

/// The worst hit in one text. Works on normalised copies: the text delivered
/// to the other side is never changed.
pub fn scan(text: &str) -> Scan {
    let norm = normalise(text);
    let chunks = digit_chunks(&norm);
    if has_phone(&chunks)
        || has_phone(&digit_chunks(&phone_form(&norm)))
        || has_account(&chunks, &norm)
        || has_phrase(&norm, OFF_PLATFORM_PHRASES)
        || LINK_MARKERS.iter().any(|m| norm.contains(m))
    {
        return OFF_PLATFORM;
    }
    if has_phrase(&norm, SCAM_PHRASES) {
        return SCAM;
    }
    let tokens = word_tokens(&norm);
    if has_word(&tokens, SERIOUS_WORDS) {
        return ABUSE;
    }
    if has_word(&tokens, MILD_WORDS) {
        return RUDE;
    }
    Scan::Clean
}

/// A phone number spread over the sender's recent messages (`recent` is oldest
/// first). The digits must touch across each message edge: the end of one
/// message and the start of the next. A word at either edge breaks the run.
/// Phones only: ten-digit runs are too easily two prices.
pub fn scan_split_digits(recent: &[&str], text: &str) -> Scan {
    let last = phone_form(&normalise(text));
    let head: Vec<&str> = last.split(' ').take_while(|t| is_digits_only(t)).collect();
    if head.is_empty() {
        return Scan::Clean;
    }
    let mut parts = Vec::new();
    for message in recent.iter().rev() {
        let message = phone_form(&normalise(message));
        let tokens: Vec<&str> = message.split(' ').collect();
        let tail: Vec<&str> = tokens
            .iter()
            .rev()
            .take_while(|t| is_digits_only(t))
            .copied()
            .collect();
        let whole = tail.len() == tokens.len();
        parts.extend(tail.into_iter().map(str::to_string));
        if !whole {
            break;
        }
    }
    if parts.is_empty() {
        return Scan::Clean;
    }
    parts.reverse();
    parts.extend(head.into_iter().map(str::to_string));
    if has_phone(&digit_chunks(&parts.join(" "))) {
        OFF_PLATFORM
    } else {
        Scan::Clean
    }
}

/// The text as a phone check should read it: look-alike letters in a token
/// that already holds a digit become digits, and "double 4" becomes "4 4".
/// Only for phone checks, which need an exact prefix and length, so a stray
/// fold cannot turn a price into a hit.
fn phone_form(norm: &str) -> String {
    let tokens: Vec<&str> = norm.split(' ').collect();
    let mut out = Vec::with_capacity(tokens.len());
    let mut repeat = 1;
    for (i, token) in tokens.iter().enumerate() {
        let times = match *token {
            "double" => 2,
            "triple" => 3,
            _ => 1,
        };
        let next_is_digit = tokens.get(i + 1).is_some_and(|t| is_digits_only(t));
        if times > 1 && next_is_digit {
            repeat = times;
            continue;
        }
        let folded = fold_look_alikes(token);
        for _ in 0..repeat {
            out.push(folded.clone());
        }
        repeat = 1;
    }
    out.join(" ")
}

fn fold_look_alikes(token: &str) -> String {
    let look_alike = |c: char| matches!(c, 'o' | 'l' | 'i');
    let fits = token.chars().any(|c| c.is_ascii_digit())
        && token
            .chars()
            .all(|c| c.is_ascii_digit() || look_alike(c) || is_digit_separator(c));
    if !fits {
        return token.to_string();
    }
    token
        .chars()
        .map(|c| match c {
            'o' => '0',
            'l' | 'i' => '1',
            c => c,
        })
        .collect()
}

/// Decimal digits NFKC leaves alone, in the scripts a dodge is likeliest to use.
fn plain_digit(c: char) -> char {
    let zero = match c {
        '\u{660}'..='\u{669}' => 0x660,
        '\u{6f0}'..='\u{6f9}' => 0x6f0,
        '\u{966}'..='\u{96f}' => 0x966,
        _ => return c,
    };
    char::from_digit(c as u32 - zero, 10).unwrap_or(c)
}

/// NFKC (full-width and styled digits become plain ones), other decimal digits
/// folded, lower-case, no zero-width characters, runs of white space become one
/// space.
fn normalise(text: &str) -> String {
    let folded: String = text
        .nfkc()
        .filter(|c| {
            !matches!(
                c,
                '\u{200b}'..='\u{200f}' | '\u{2060}' | '\u{feff}' | '\u{ad}'
            )
        })
        .map(plain_digit)
        .flat_map(char::to_lowercase)
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_digits_only(norm: &str) -> bool {
    !norm.is_empty()
        && norm
            .split(|c: char| is_digit_separator(c) || c == ' ')
            .filter(|t| !t.is_empty())
            .all(|t| t.chars().all(|c| c.is_ascii_digit()) || number_word(t).is_some())
}

fn number_word(word: &str) -> Option<char> {
    Some(match word {
        "zero" | "oh" => '0',
        "one" => '1',
        "two" => '2',
        "three" => '3',
        "four" => '4',
        "five" => '5',
        "six" => '6',
        "seven" => '7',
        "eight" => '8',
        "nine" => '9',
        _ => return None,
    })
}

fn is_digit_separator(c: char) -> bool {
    matches!(c, '-' | '.' | '(' | ')' | '+' | ',' | '/' | '_')
}

/// How a group of digits is joined to the group before it.
#[derive(Clone, Copy, PartialEq)]
enum Join {
    /// The first group of a run: nothing before it.
    Start,
    /// Touching, or `-` and `.`: one written number.
    Tight,
    /// White space, a comma or a slash: may be several numbers.
    Loose,
}

struct Chunk {
    digits: String,
    join: Join,
}

/// The digit groups of a text, in order. Each number word is a group of one.
/// A word that is not a number, or any other character, ends the run.
fn digit_chunks(norm: &str) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut join = Join::Start;
    let mut digits = String::new();
    let mut word = String::new();
    // A trailing NUL flushes the last group.
    for c in norm.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_digit() && word.is_empty() {
            digits.push(c);
            continue;
        }
        if c.is_alphabetic() || c.is_ascii_digit() {
            if !digits.is_empty() {
                // Letters glued to digits: not a number written out (`256gb`).
                digits.clear();
                join = Join::Start;
            }
            word.push(c);
            continue;
        }
        if !digits.is_empty() {
            chunks.push(Chunk {
                digits: std::mem::take(&mut digits),
                join,
            });
            join = Join::Tight;
        } else if !word.is_empty() {
            match number_word(&word) {
                Some(d) => {
                    chunks.push(Chunk {
                        digits: d.to_string(),
                        join,
                    });
                    join = Join::Tight;
                }
                None => join = Join::Start,
            }
            word.clear();
        }
        join = match (c, join) {
            (_, Join::Start) => Join::Start,
            (' ' | ',' | '/', _) => Join::Loose,
            (c, j) if is_digit_separator(c) => j,
            _ => Join::Start,
        };
    }
    chunks
}

/// Every run of whole groups, with whether it has a loose join inside. A
/// number must start and end on a group edge, so a row of prices is never
/// searched for phone numbers inside it.
fn windows(chunks: &[Chunk]) -> impl Iterator<Item = (String, bool)> + '_ {
    (0..chunks.len()).flat_map(move |start| {
        let mut joined = String::new();
        let mut loose = false;
        (start..chunks.len()).map_while(move |end| {
            if end > start {
                match chunks[end].join {
                    Join::Start => return None,
                    Join::Loose => loose = true,
                    Join::Tight => {}
                }
            }
            joined.push_str(&chunks[end].digits);
            (joined.len() <= 13).then(|| (joined.clone(), loose))
        })
    })
}

fn has_phone(chunks: &[Chunk]) -> bool {
    let local = |d: &str| NIGERIAN_PREFIXES.iter().any(|p| d.starts_with(p));
    windows(chunks).any(|(d, _)| {
        (d.len() == 11 && d.starts_with('0') && local(&d[1..]))
            || (d.len() == 13 && d.starts_with("234") && local(&d[3..]))
    })
}

fn has_account(chunks: &[Chunk], norm: &str) -> bool {
    let bank_word = BANK_WORDS.iter().any(|w| norm.contains(w));
    windows(chunks).any(|(d, loose)| d.len() == 10 && (!loose || bank_word))
}

/// A phrase in the text, on word edges: `wa.me` must not match `kawa.me`.
fn has_phrase(norm: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| {
        norm.match_indices(p).any(|(at, _)| {
            let before = norm[..at].chars().next_back();
            let after = norm[at + p.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

/// Words with look-alike digits and symbols mapped back to letters, and
/// letters written one at a time (`m u m u`, `m.u.m.u`) joined up.
fn word_tokens(norm: &str) -> Vec<String> {
    let mapped: String = norm
        .chars()
        .map(|c| match c {
            '1' => 'i',
            '3' => 'e',
            '4' => 'a',
            '0' => 'o',
            '@' => 'a',
            '$' => 's',
            other => other,
        })
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut letters = String::new();
    for token in mapped
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        if token.chars().count() == 1 {
            letters.push_str(token);
            continue;
        }
        if !letters.is_empty() {
            out.push(std::mem::take(&mut letters));
        }
        out.push(token.to_string());
    }
    if !letters.is_empty() {
        out.push(letters);
    }
    out
}

fn squeeze(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if !out.ends_with(c) {
            out.push(c);
        }
    }
    out
}

/// A stretched spelling matches (`muuumu`), a shorter one never does: `as`
/// squeezes to the same letters as `ass` but is not that word.
fn same_word(token: &str, word: &str) -> bool {
    token == word || (token.len() >= word.len() && squeeze(token) == squeeze(word))
}

fn has_word(tokens: &[String], list: &[&str]) -> bool {
    list.iter().any(|entry| {
        let parts: Vec<&str> = entry.split(' ').collect();
        tokens
            .windows(parts.len())
            .any(|w| w.iter().zip(&parts).all(|(t, p)| same_word(t, p)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFF: Scan = Scan::Hit {
        severity: Severity::Serious,
        category: Category::OffPlatform,
    };
    const RUDE: Scan = Scan::Hit {
        severity: Severity::Mild,
        category: Category::Rude,
    };
    const ABUSE: Scan = Scan::Hit {
        severity: Severity::Serious,
        category: Category::Abuse,
    };

    #[test]
    fn phone_variants_are_caught() {
        for text in [
            "0803 123 4567",
            "0803-123-4567",
            "08031234567",
            "call 0803.123.4567 now",
            "+234 803 123 4567",
            "2348031234567",
            "(0803) 123 4567",
            "zero eight zero three one two three four five six seven",
            "０８０３１２３４５６７",
            "0803\u{200b}1234567",
        ] {
            assert_eq!(scan(text), OFF, "{text}");
        }
    }

    #[test]
    fn disguised_digits_are_caught() {
        for text in [
            "O8O3 123 4567",
            "0803l234567",
            "o8o3-l23-4567",
            "\u{660}\u{668}\u{660}\u{663}\u{661}\u{662}\u{663}\u{664}\u{665}\u{666}\u{667}",
            "zero eight zero three one two three double four five six",
            "0 8 0 3 triple one 4 5 6 7",
        ] {
            assert_eq!(scan(text), OFF, "{text}");
        }
    }

    #[test]
    fn look_alike_folding_leaves_words_alone() {
        for text in [
            "lol i will take it",
            "oil 5000 litres 1000",
            "double 5000 is too much",
            "size 10 or 11",
            "l2 and o3 are fine",
        ] {
            assert_eq!(scan(text), Scan::Clean, "{text}");
        }
    }

    #[test]
    fn account_numbers_are_caught() {
        assert_eq!(scan("0123456789"), OFF);
        assert_eq!(scan("0123-456-789"), OFF);
        assert_eq!(scan("my account is 012 345 6789"), OFF);
        assert_eq!(scan("012 345 6789"), Scan::Clean);
    }

    #[test]
    fn prices_are_not_phones() {
        for text in [
            "5000 7000 8000",
            "8000 9000 7000 1000",
            "₦1,500,000",
            "size 42",
            "iPhone 13 Pro 256GB",
            "2 for 5000",
            "I can pay 15000",
        ] {
            assert_eq!(scan(text), Scan::Clean, "{text}");
        }
    }

    #[test]
    fn off_platform_phrases_block() {
        for text in [
            "chat me on whatsapp",
            "WhatsApp me",
            "wa.me/2348031234567",
            "t.me/shop",
            "pay me directly",
            "make we talk for whatsapp",
            "drop your number",
            "see http://shop.example",
            "see https://shop.example",
            "go to www.shop.example",
        ] {
            assert_eq!(scan(text), OFF, "{text}");
        }
    }

    #[test]
    fn scam_phrases_block() {
        assert_eq!(
            scan("please send your otp"),
            Scan::Hit {
                severity: Severity::Serious,
                category: Category::Scam
            }
        );
    }

    #[test]
    fn wallet_names_alone_are_clean() {
        assert_eq!(scan("can I pay with opay?"), Scan::Clean);
    }

    #[test]
    fn word_filter_sees_through_disguises() {
        for text in ["MUMU", "muuumu", "m u m u", "m.u.m.u", "you be mumu!"] {
            assert_eq!(scan(text), RUDE, "{text}");
        }
        assert_eq!(scan("you are an onye ara"), RUDE);
        assert_eq!(scan("ash4wo"), ABUSE);
    }

    #[test]
    fn innocent_words_are_clean() {
        for text in [
            "assess",
            "class",
            "Scunthorpe",
            "ode to joy",
            "grass cutter",
            "as you like",
            "a b c",
        ] {
            assert_eq!(scan(text), Scan::Clean, "{text}");
        }
    }

    #[test]
    fn serious_beats_mild() {
        assert_eq!(scan("mumu ashawo"), ABUSE);
        assert_eq!(scan("mumu call 08031234567"), OFF);
    }

    #[test]
    fn split_digits() {
        assert_eq!(scan_split_digits(&["0803", "123"], "4567"), OFF);
        assert_eq!(
            scan_split_digits(&["how much?", "0803"], "4567"),
            Scan::Clean
        );
        assert_eq!(scan_split_digits(&["5000", "7000"], "8000"), Scan::Clean);
        assert_eq!(scan_split_digits(&["0803", "123"], "okay"), Scan::Clean);
        assert_eq!(scan_split_digits(&[], "4567"), Scan::Clean);
    }

    #[test]
    fn split_digits_with_words_around_them() {
        assert_eq!(scan_split_digits(&["call 0803 123"], "4567"), OFF);
        assert_eq!(scan_split_digits(&["my line is 0803"], "1234567 ok"), OFF);
        assert_eq!(scan_split_digits(&["call 0803", "123"], "4567 today"), OFF);
        assert_eq!(scan_split_digits(&["O8O3"], "l234567"), OFF);
        // Digits that do not touch across the message edge stay apart.
        assert_eq!(scan_split_digits(&["0803 please"], "1234567"), Scan::Clean);
        assert_eq!(scan_split_digits(&["0803 123"], "for 4567"), Scan::Clean);
        assert_eq!(
            scan_split_digits(&["I paid 8000"], "1000 more"),
            Scan::Clean
        );
    }

    #[test]
    fn category_names_match_the_stored_values() {
        assert_eq!(Category::OffPlatform.as_str(), "off_platform");
        assert_eq!(Category::Unreviewable.as_str(), "unreviewable");
    }
}
