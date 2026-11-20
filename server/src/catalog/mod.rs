//! The public catalog the app browses: categories (task 1), search and browse
//! (task 2) and the store directory (task 3). Read-only SQL over products,
//! variants and stores; nothing here changes state.

pub mod browse;
pub mod categories;
pub mod stores;
