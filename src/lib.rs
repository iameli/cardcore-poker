extern crate alloc;

pub mod agent;
pub mod card;
#[cfg(target_arch = "wasm32")]
pub mod wasm;
pub mod crypto;
pub mod error;
pub mod eval;
pub mod game;
pub mod lexicon;
pub mod protocol;

pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>;
