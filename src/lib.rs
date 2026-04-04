pub mod card;
pub mod crypto;
pub mod error;
pub mod eval;
pub mod game;
pub mod protocol;

pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>;
