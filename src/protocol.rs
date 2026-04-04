//! Mental poker protocol state machine.
//!
//! The protocol progresses through phases, each requiring specific actions
//! from specific players. `valid_actions()` returns what's currently expected,
//! making it easy to fuzz test by randomly picking valid actions.

use serde::{Deserialize, Serialize};

use crate::crypto::{self, Point, Scalar};
use crate::game::{BetAction, GameState, PlayerId};

/// The phase of the mental poker protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Waiting for players to join.
    WaitingForPlayers { need: usize },
    /// Players commit their RNG seeds (hashed).
    CommitSeeds,
    /// Players reveal their RNG seeds.
    RevealSeeds,
    /// Players take turns encrypting and shuffling the deck.
    Shuffle { next_player: PlayerId },
    /// Players provide decryption shares to deal hole cards.
    DealHole { for_player: PlayerId, card_idx: usize },
    /// Betting round.
    Betting,
    /// Players provide decryption shares for community cards.
    DealCommunity { num_to_deal: usize },
    /// Players reveal hole cards for showdown.
    Showdown,
    /// Hand is complete.
    Complete,
}

/// An action that can be taken in the current protocol state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    /// Join the game.
    Join { player_id: PlayerId },
    /// Commit a hashed RNG seed.
    CommitSeed {
        player_id: PlayerId,
        commitment: [u8; crypto::HASH_BYTES],
    },
    /// Reveal the RNG seed (must match prior commitment).
    RevealSeed {
        player_id: PlayerId,
        seed: Vec<u8>,
    },
    /// Encrypt all cards with per-position keys and shuffle.
    ShuffleDeck {
        player_id: PlayerId,
        deck: Vec<Point>,
    },
    /// Provide a decryption scalar for a specific card position.
    DecryptCard {
        player_id: PlayerId,
        position: usize,
        scalar: Scalar,
    },
    /// Betting action.
    Bet {
        player_id: PlayerId,
        action: BetAction,
    },
    /// Reveal hole cards at showdown.
    RevealHand {
        player_id: PlayerId,
        scalars: Vec<Scalar>,
    },
}

impl Action {
    pub fn player_id(&self) -> PlayerId {
        match self {
            Action::Join { player_id }
            | Action::CommitSeed { player_id, .. }
            | Action::RevealSeed { player_id, .. }
            | Action::ShuffleDeck { player_id, .. }
            | Action::DecryptCard { player_id, .. }
            | Action::Bet { player_id, .. }
            | Action::RevealHand { player_id, .. } => *player_id,
        }
    }
}

/// Description of what actions are currently valid. Used for validation and fuzz testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidAction {
    pub player_id: PlayerId,
    pub kind: ValidActionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidActionKind {
    Join,
    CommitSeed,
    RevealSeed,
    ShuffleDeck,
    /// Decrypt card at this position.
    DecryptCard { position: usize },
    /// Valid betting actions for this player.
    Bet { options: Vec<BetAction> },
    RevealHand,
}

/// The full protocol state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolState {
    pub phase: Phase,
    pub game: GameState,
    pub seed_commitments: Vec<Option<[u8; crypto::HASH_BYTES]>>,
    pub seeds_revealed: Vec<Option<Vec<u8>>>,
    pub combined_seed: Option<[u8; crypto::HASH_BYTES]>,
    /// Track which players have shuffled.
    pub shuffles_done: usize,
}

impl ProtocolState {
    pub fn new(num_players: usize, starting_chips: u64, small_blind: u64) -> Self {
        Self {
            phase: Phase::WaitingForPlayers {
                need: num_players,
            },
            game: GameState::new(num_players, starting_chips, small_blind),
            seed_commitments: vec![None; num_players],
            seeds_revealed: vec![None; num_players],
            combined_seed: None,
            shuffles_done: 0,
        }
    }

    /// Returns all actions that are valid in the current state.
    /// This is the key function for fuzz testing — randomly pick from this list.
    pub fn valid_actions(&self) -> Vec<ValidAction> {
        match &self.phase {
            Phase::WaitingForPlayers { need } => {
                // Any player slot that hasn't joined yet
                (0..self.game.num_players())
                    .filter(|_| *need > 0)
                    .map(|pid| ValidAction {
                        player_id: pid,
                        kind: ValidActionKind::Join,
                    })
                    .collect()
            }
            Phase::CommitSeeds => self
                .seed_commitments
                .iter()
                .enumerate()
                .filter(|(_, c)| c.is_none())
                .map(|(pid, _)| ValidAction {
                    player_id: pid,
                    kind: ValidActionKind::CommitSeed,
                })
                .collect(),
            Phase::RevealSeeds => self
                .seeds_revealed
                .iter()
                .enumerate()
                .filter(|(_, s)| s.is_none())
                .map(|(pid, _)| ValidAction {
                    player_id: pid,
                    kind: ValidActionKind::RevealSeed,
                })
                .collect(),
            Phase::Shuffle { next_player } => {
                vec![ValidAction {
                    player_id: *next_player,
                    kind: ValidActionKind::ShuffleDeck,
                }]
            }
            Phase::DealHole { for_player, card_idx } => {
                // Every player except the recipient needs to provide their decryption share
                (0..self.game.num_players())
                    .filter(|pid| *pid != *for_player)
                    .map(|pid| ValidAction {
                        player_id: pid,
                        kind: ValidActionKind::DecryptCard {
                            position: *card_idx,
                        },
                    })
                    .collect()
            }
            Phase::Betting => {
                if let Some(pid) = self.game.action_on {
                    let options = self.valid_bet_actions(pid);
                    vec![ValidAction {
                        player_id: pid,
                        kind: ValidActionKind::Bet { options },
                    }]
                } else {
                    vec![]
                }
            }
            Phase::DealCommunity { num_to_deal: _ } => {
                // All players provide decryption shares for the next community card
                let next_pos = self.next_community_deck_position();
                (0..self.game.num_players())
                    .map(|pid| ValidAction {
                        player_id: pid,
                        kind: ValidActionKind::DecryptCard { position: next_pos },
                    })
                    .collect()
            }
            Phase::Showdown => self
                .game
                .players
                .iter()
                .enumerate()
                .filter(|(_, p)| !p.folded)
                .map(|(pid, _)| ValidAction {
                    player_id: pid,
                    kind: ValidActionKind::RevealHand,
                })
                .collect(),
            Phase::Complete => vec![],
        }
    }

    fn valid_bet_actions(&self, player_id: PlayerId) -> Vec<BetAction> {
        let player = &self.game.players[player_id];
        let to_call = self.game.current_bet.saturating_sub(player.bet_this_street);
        let mut actions = Vec::new();

        if to_call == 0 {
            actions.push(BetAction::Check);
        } else if player.chips >= to_call {
            actions.push(BetAction::Call);
        }

        if to_call > 0 || self.game.current_bet == 0 {
            actions.push(BetAction::Fold);
        }

        // Can raise if you have enough chips
        let min_raise = self.game.big_blind;
        if player.chips > to_call + min_raise {
            actions.push(BetAction::Raise(to_call + min_raise));
        }

        if player.chips > 0 {
            actions.push(BetAction::AllIn);
        }

        actions
    }

    /// Which deck position is the next community card?
    /// Hole cards come first (2 per player), then community cards.
    fn next_community_deck_position(&self) -> usize {
        let hole_card_count = self.game.num_players() * 2;
        hole_card_count + self.game.community.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_valid_actions() {
        let state = ProtocolState::new(2, 1000, 10);
        let actions = state.valid_actions();
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0].kind, ValidActionKind::Join));
    }
}
