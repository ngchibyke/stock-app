//! Hard limits shared by request validation and the database CHECK constraints
//! (`migrations/0007_carry_forward.sql` repeats these literals; a test keeps them in step).

/// Largest unit price (product base price or variant override): NGN 10m in kobo.
pub const MAX_PRICE_KOBO: i64 = 1_000_000_000;
/// Largest stock level of a single variant at one branch.
pub const MAX_STOCK: i32 = 100_000;
/// Most open branches one store may have. A variant's total stock is capped at this
/// many times [`MAX_STOCK`] (`migrations/0046_variant_stock_totals.sql`).
pub const MAX_OPEN_BRANCHES: i64 = 20;
/// Largest quantity of one cart line (enforced when carts are added).
pub const MAX_LINE_QUANTITY: i32 = 100;
/// Largest checkout total in kobo (NGN 50m).
pub const MAX_CHECKOUT_TOTAL_KOBO: i64 = 5_000_000_000;
/// Longest chat message, in characters.
pub const MAX_CHAT_CHARS: usize = 1000;
/// Longest review text, in characters.
pub const MAX_REVIEW_CHARS: usize = 2000;
/// Longest seller reply to a review, in characters.
pub const MAX_REPLY_CHARS: usize = 1000;
/// Most photos on one review.
pub const MAX_REVIEW_PHOTOS: usize = 5;
/// Chat messages one user may send per minute.
pub const CHAT_SENDS_PER_MINUTE: i64 = 20;
/// Chat messages one user may send per day.
pub const CHAT_SENDS_PER_DAY: i64 = 500;
/// Conversations one user may start per day.
pub const NEW_CONVERSATIONS_PER_DAY: i64 = 20;
/// Longest dispute statement, in characters (after trimming).
pub const MAX_DISPUTE_CHARS: usize = 2000;
/// Most photos on one dispute statement.
pub const MAX_DISPUTE_PHOTOS: usize = 5;

// ---- Photo uploads (plan 7a) ----

/// Largest raw upload, in bytes.
pub const MAX_UPLOAD_BYTES: i64 = 5 * 1024 * 1024;
/// Longest side of an uploaded photo, in pixels. (4096 x 4096 is under 17 million pixels.)
pub const MAX_UPLOAD_EDGE: u32 = 4096;
/// Longest side of a stored photo: bigger ones are scaled down.
pub const OUTPUT_LONGEST_EDGE: u32 = 2048;
/// JPEG quality of a stored photo.
pub const OUTPUT_JPEG_QUALITY: u8 = 85;
/// Upload slots one person may ask for per hour.
pub const UPLOADS_PER_HOUR: i64 = 30;
/// How long a signed upload URL lives, in seconds.
pub const UPLOAD_URL_TTL_SECS: u64 = 600;
/// How long a signed link to a dispute photo works.
pub const EVIDENCE_URL_TTL_SECS: u64 = 60;
/// A pending or failed upload older than this is deleted, with its object.
pub const STALE_UPLOAD_HOURS: i32 = 24;
