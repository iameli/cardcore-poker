#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid card index: {0}")]
    InvalidCard(u8),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("invalid state transition: {0}")]
    InvalidAction(String),
    /// The action isn't valid *yet* — it may become valid as other actions
    /// arrive (turn order, phase progression). During a concurrent multi-repo
    /// replay this is routine: buffer and retry. Everything else an authored
    /// record can fail with is a protocol VIOLATION — the author published an
    /// action that can never be valid, i.e. cheating or a broken client.
    #[error("out of order: {0}")]
    OutOfOrder(String),
}
