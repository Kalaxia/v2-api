pub mod auth;
pub mod error;
pub mod pagination;
pub mod time;

/// Helper type used as a return type for HTTP handler.
/// This type helps agregating multiple error types from this crate as well as different external
/// crates which have an error system.
pub type Result<T> = std::result::Result<T, error::ServerError>;
