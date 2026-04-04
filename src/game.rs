//! Hold'em game state and betting logic.

use serde::{Deserialize, Serialize};

use crate::crypto::Point;

/// Identifies a player by their index in the game (0-based, in seat order).
pub type PlayerId = usize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
    Showdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BetAction {
    Fold,
    Check,
    Call,
    Raise(u64),
    AllIn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub chips: u64,
    pub bet_this_street: u64,
    pub folded: bool,
    pub all_in: bool,
    /// Encrypted hole cards (2 points, only known to this player until showdown).
    pub hole_encrypted: Vec<Point>,
    /// Decrypted hole cards (filled in once all other players provide decryption shares).
    pub hole_points: Vec<Point>,
}

impl PlayerState {
    pub fn new(chips: u64) -> Self {
        Self {
            chips,
            bet_this_street: 0,
            folded: false,
            all_in: false,
            hole_encrypted: Vec::new(),
            hole_points: Vec::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        !self.folded && !self.all_in
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub players: Vec<PlayerState>,
    pub street: Street,
    pub pot: u64,
    pub current_bet: u64,
    /// Index of the dealer button.
    pub button: usize,
    /// Whose turn it is to act (for betting).
    pub action_on: Option<PlayerId>,
    /// The encrypted deck after all players have shuffled.
    pub deck: Vec<Point>,
    /// Community card points (revealed progressively).
    pub community: Vec<Point>,
    /// Small blind amount.
    pub small_blind: u64,
    /// Big blind amount.
    pub big_blind: u64,
    /// Number of players who have acted this betting round.
    pub actions_this_round: usize,
}

impl GameState {
    pub fn new(num_players: usize, starting_chips: u64, small_blind: u64) -> Self {
        assert!(num_players >= 2 && num_players <= 10);
        Self {
            players: (0..num_players)
                .map(|_| PlayerState::new(starting_chips))
                .collect(),
            street: Street::Preflop,
            pot: 0,
            current_bet: 0,
            button: 0,
            action_on: None,
            deck: Vec::new(),
            community: Vec::new(),
            small_blind,
            big_blind: small_blind * 2,
            actions_this_round: 0,
        }
    }

    pub fn num_players(&self) -> usize {
        self.players.len()
    }

    /// Players still in the hand (not folded).
    pub fn active_player_count(&self) -> usize {
        self.players.iter().filter(|p| !p.folded).count()
    }

    /// Players who can still act (not folded, not all-in).
    pub fn actionable_player_count(&self) -> usize {
        self.players.iter().filter(|p| p.is_active()).count()
    }
}
