//! Blackjack: rules core, protocol state machine, and agent.
//!
//! European no-hole-card (ENHC) blackjack with a rotating banker. Every card
//! is dealt face-up through the shared mental-card engine — no hidden state.

pub mod agent;
pub mod eval;
pub mod game;
pub mod protocol;
