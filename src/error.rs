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
}
