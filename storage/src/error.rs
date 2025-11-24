//! Error types for the crate

#[derive(Debug)]
/// Custom error type
pub enum Error {
    // Flash memory errors (non-volatile memory)
    /// Flash write error
    FlashWriteError,
    /// Flash read error
    FlashReadError,
}

/// Result type using the common errors for the workspace
pub type Result<T> = core::result::Result<T, Error>;
