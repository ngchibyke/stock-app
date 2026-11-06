//! Seed word lists, lower-case. A starting point for native-speaker review
//! (roadmap go-live gate), not a finished vocabulary. Words that are also
//! ordinary English or ordinary trade terms stay out (`ode`, `were`, `cock`,
//! `ewu`): a false strike is worse than leaving those to the tone check.
//!
//! Entries may hold several words. Matching is on whole words only.

/// Slurs, sexual content and open threats: blocked, with a strike.
pub const SERIOUS_WORDS: &[&str] = &[
    // English
    "nigger",
    "faggot",
    "whore",
    "pussy",
    "blowjob",
    "send nudes",
    "i will kill you",
    "i will find you",
    "i will rape you",
    // Pidgin
    "ashawo",
    "ashewo",
    "olosho",
    "i go kill you",
    "i go finish you",
    "i go deal with you",
    // Hausa
    "karuwa",
    "zan kashe ka",
    "zan kashe ki",
    // Yoruba
    "maa pa e",
    "omo ale",
    // Igbo
    "aga m egbu gi",
];

/// Rude words: the sender is offered a nicer version.
pub const MILD_WORDS: &[&str] = &[
    // English
    "idiot",
    "stupid",
    "fool",
    "bastard",
    "fuck",
    "shit",
    "asshole",
    "bitch",
    // Pidgin
    "mumu",
    "mugu",
    "olodo",
    "werey",
    "yeye",
    // Hausa
    "wawa",
    "shege",
    "dan iska",
    "dan banza",
    "mahaukaci",
    // Yoruba
    "oloshi",
    "oponu",
    "didinrin",
    "agbaya",
    // Igbo
    "onye ara",
    "ofeke",
    "anuofia",
];

/// Moving the contact or the payment out of the app.
pub const OFF_PLATFORM_PHRASES: &[&str] = &[
    "whatsapp",
    "whatapp",
    "whatsap",
    "watsapp",
    "wa.me",
    "telegram",
    "t.me",
    "bit.ly",
    "pay me directly",
    "pay directly",
    "pay outside",
    "outside the app",
    "outside this app",
    "off the app",
    "direct transfer",
    "send to my account",
    "pay into my account",
    "my account number",
    "my acct",
    "call my line",
    "call me on",
    "text me on",
    "dm me",
    "drop your number",
    "send your number",
    "give me your number",
    "wetin be your number",
    "send me your number",
    "abeg call me",
];

/// Deception that needs no change of platform.
pub const SCAM_PHRASES: &[&str] = &[
    "send your otp",
    "give me your otp",
    "share your otp",
    "the otp you received",
    "send your bvn",
    "give me your bvn",
    "your atm pin",
    "your card pin",
    "your handover code",
    "confirm before you receive",
    "confirm before delivery",
    "click received now",
];

/// Parts of a web address. Links always count as leaving the app.
pub const LINK_MARKERS: &[&str] = &["http://", "https://", "www."];
