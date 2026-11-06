#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("builder missing required field: {0}")]
    BuilderMissingField(&'static str),
    #[error("MOO protocol error: {0}")]
    Moo(#[from] roon_moo::MooError),
    #[error("SOOD discovery error: {0}")]
    Sood(#[from] roon_sood::SoodError),
    #[error("registry handshake failed: {0}")]
    RegistryFailed(String),
    #[error("not connected to any core")]
    NotConnected,
    #[error("connection closed")]
    ConnectionClosed,
    #[error("I/O error: {0}")]
    Io(String),
}
