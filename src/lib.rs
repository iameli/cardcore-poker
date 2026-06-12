extern crate alloc;

pub mod agent;
pub mod agent_util;
pub mod blackjack;
pub mod card;
pub mod crypto;
pub mod engine;
pub mod error;
pub mod eval;
pub mod game;
pub mod lexicon;
pub mod protocol;
pub mod sim;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>;
