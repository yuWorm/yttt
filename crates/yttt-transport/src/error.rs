#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("transport connection is closed")]
    Closed,
    #[error("transport I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("transport failure is permanent: {0}")]
    Permanent(String),
    #[error("transport failed: {0}")]
    Other(String),
}
