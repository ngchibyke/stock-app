//! Hard limits shared by request validation and the database CHECK constraints
//! (`migrations/0007_carry_forward.sql` repeats these literals; a test keeps them in step).

/// Largest unit price (product base price or variant override): NGN 10m in kobo.
pub const MAX_PRICE_KOBO: i64 = 1_000_000_000;
/// Largest stock level of a single variant.
pub const MAX_STOCK: i32 = 100_000;
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
