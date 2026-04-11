//! Mental poker protocol state machine.
//!
//! Two-phase shuffle+lock protocol:
//! 1. Shuffle: each player encrypts all cards with one key, shuffles.
//! 2. Lock: each player removes their shuffle key, re-encrypts with per-position keys.
//! 3. Deal: players reveal per-position lock scalars. Verifiable by anyone.
//!
//! `valid_actions()` returns what's currently expected — for fuzz testing,
//! randomly pick valid actions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::crypto::{self, Point, Scalar};
use crate::game::{BetAction, GameState, PlayerId, Street};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    WaitingForPlayers { need: usize },
    CommitSeeds,
    RevealSeeds,
    /// Players take turns encrypting and shuffling the deck.
    Shuffle { next_player: PlayerId },
    /// Players take turns removing shuffle encryption and adding per-position lock keys.
    Lock { next_player: PlayerId },
    /// Players reveal per-position lock scalars to deal a card.
    Dealing {
        deal_type: DealType,
        deck_position: usize,
    },
    Betting,
    Showdown,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DealType {
    HoleCard {
        for_player: PlayerId,
        card_idx: usize,
    },
    CommunityCard {
        remaining_this_street: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    Join { player_id: PlayerId },
    CommitSeed {
        player_id: PlayerId,
        commitment: [u8; crypto::HASH_BYTES],
    },
    RevealSeed {
        player_id: PlayerId,
        seed: Vec<u8>,
    },
    /// Shuffle phase: encrypt all cards with shuffle key and shuffle.
    ShuffleDeck {
        player_id: PlayerId,
        deck: Vec<Point>,
    },
    /// Lock phase: remove shuffle encryption, apply per-position lock keys.
    LockDeck {
        player_id: PlayerId,
        deck: Vec<Point>,
    },
    /// Reveal a per-position lock scalar for dealing.
    RevealLockKey {
        player_id: PlayerId,
        deck_position: usize,
        scalar: Scalar,
    },
    Bet {
        player_id: PlayerId,
        action: BetAction,
    },
    /// Reveal lock keys for hole card positions at showdown.
    RevealHand {
        player_id: PlayerId,
        /// Lock scalars for this player's hole card positions.
        scalars: Vec<(usize, Scalar)>,
    },
}

impl Action {
    pub fn player_id(&self) -> PlayerId {
        match self {
            Action::Join { player_id }
            | Action::CommitSeed { player_id, .. }
            | Action::RevealSeed { player_id, .. }
            | Action::ShuffleDeck { player_id, .. }
            | Action::LockDeck { player_id, .. }
            | Action::RevealLockKey { player_id, .. }
            | Action::Bet { player_id, .. }
            | Action::RevealHand { player_id, .. } => *player_id,
        }
    }
}

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
    LockDeck,
    /// Reveal lock key for this deck position.
    RevealLockKey { deck_position: usize },
    Bet { options: Vec<BetAction> },
    RevealHand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolState {
    pub phase: Phase,
    pub game: GameState,
    pub seed_commitments: Vec<Option<[u8; crypto::HASH_BYTES]>>,
    pub seeds_revealed: Vec<Option<Vec<u8>>>,
    pub combined_seed: Option<[u8; crypto::HASH_BYTES]>,
    pub shuffles_done: usize,
    pub locks_done: usize,
    pub showdown_revealed: Vec<bool>,
    pub next_deck_position: usize,
    hole_deal_queue: Vec<(PlayerId, usize)>,
    /// Track which players have revealed their lock key for the current deal.
    pub deal_reveals: HashMap<PlayerId, Scalar>,
    /// For each player, the deck positions of their hole cards.
    pub hole_card_positions: Vec<Vec<usize>>,
}

impl ProtocolState {
    pub fn new(num_players: usize, starting_chips: u64, small_blind: u64) -> Self {
        Self {
            phase: Phase::WaitingForPlayers { need: num_players },
            game: GameState::new(num_players, starting_chips, small_blind),
            seed_commitments: vec![None; num_players],
            seeds_revealed: vec![None; num_players],
            combined_seed: None,
            shuffles_done: 0,
            locks_done: 0,
            showdown_revealed: vec![false; num_players],
            next_deck_position: 0,
            hole_deal_queue: Vec::new(),
            deal_reveals: HashMap::new(),
            hole_card_positions: vec![Vec::new(); num_players],
        }
    }

    pub fn apply(&mut self, action: &Action) -> crate::Result<()> {
        match (&self.phase, action) {
            // --- Join ---
            (Phase::WaitingForPlayers { need }, Action::Join { player_id }) => {
                if *player_id >= self.game.num_players() {
                    return Err(crate::Error::InvalidAction("invalid player id".into()));
                }
                let remaining = need - 1;
                if remaining == 0 {
                    self.phase = Phase::CommitSeeds;
                } else {
                    self.phase = Phase::WaitingForPlayers { need: remaining };
                }
                Ok(())
            }

            // --- Commit Seeds ---
            (Phase::CommitSeeds, Action::CommitSeed {
                player_id,
                commitment,
            }) => {
                if self.seed_commitments[*player_id].is_some() {
                    return Err(crate::Error::InvalidAction("already committed".into()));
                }
                self.seed_commitments[*player_id] = Some(*commitment);
                if self.seed_commitments.iter().all(|c| c.is_some()) {
                    self.phase = Phase::RevealSeeds;
                }
                Ok(())
            }

            // --- Reveal Seeds ---
            (Phase::RevealSeeds, Action::RevealSeed { player_id, seed }) => {
                if self.seeds_revealed[*player_id].is_some() {
                    return Err(crate::Error::InvalidAction("already revealed".into()));
                }
                let hash = crypto::blake2b(seed)?;
                let commitment = self.seed_commitments[*player_id]
                    .ok_or_else(|| crate::Error::InvalidAction("no commitment found".into()))?;
                if hash != commitment {
                    return Err(crate::Error::Crypto("seed doesn't match commitment".into()));
                }
                self.seeds_revealed[*player_id] = Some(seed.clone());

                if self.seeds_revealed.iter().all(|s| s.is_some()) {
                    let mut combined = Vec::new();
                    for s in &self.seeds_revealed {
                        combined.extend_from_slice(s.as_ref().unwrap());
                    }
                    self.combined_seed = Some(crypto::blake2b(&combined)?);

                    let card_points = crypto::card_points()?;
                    self.game.deck = card_points.into_iter().map(|(_, p)| p).collect();
                    self.phase = Phase::Shuffle { next_player: 0 };
                }
                Ok(())
            }

            // --- Shuffle ---
            (Phase::Shuffle { next_player }, Action::ShuffleDeck { player_id, deck }) => {
                if *player_id != *next_player {
                    return Err(crate::Error::InvalidAction("not your turn to shuffle".into()));
                }
                if deck.len() != 52 {
                    return Err(crate::Error::InvalidAction("deck must have 52 cards".into()));
                }
                self.game.deck = deck.clone();
                self.shuffles_done += 1;

                if self.shuffles_done >= self.game.num_players() {
                    // All players shuffled — move to lock phase
                    self.phase = Phase::Lock { next_player: 0 };
                } else {
                    self.phase = Phase::Shuffle {
                        next_player: self.shuffles_done,
                    };
                }
                Ok(())
            }

            // --- Lock ---
            (Phase::Lock { next_player }, Action::LockDeck { player_id, deck }) => {
                if *player_id != *next_player {
                    return Err(crate::Error::InvalidAction("not your turn to lock".into()));
                }
                if deck.len() != 52 {
                    return Err(crate::Error::InvalidAction("deck must have 52 cards".into()));
                }
                self.game.deck = deck.clone();
                self.locks_done += 1;

                if self.locks_done >= self.game.num_players() {
                    // All players locked — post blinds and start dealing
                    self.game.post_blinds();
                    self.start_dealing_hole_cards();
                } else {
                    self.phase = Phase::Lock {
                        next_player: self.locks_done,
                    };
                }
                Ok(())
            }

            // --- Reveal Lock Key (for dealing) ---
            (
                Phase::Dealing {
                    deal_type,
                    deck_position,
                },
                Action::RevealLockKey {
                    player_id,
                    deck_position: action_pos,
                    scalar,
                },
            ) => {
                if *action_pos != *deck_position {
                    return Err(crate::Error::InvalidAction("wrong deck position".into()));
                }

                // Check this player should be revealing
                let exclude = match deal_type {
                    DealType::HoleCard { for_player, .. } => Some(*for_player),
                    DealType::CommunityCard { .. } => None,
                };
                if exclude == Some(*player_id) {
                    return Err(crate::Error::InvalidAction(
                        "recipient doesn't reveal for their own card".into(),
                    ));
                }
                if self.deal_reveals.contains_key(player_id) {
                    return Err(crate::Error::InvalidAction("already revealed".into()));
                }

                self.deal_reveals.insert(*player_id, scalar.clone());

                // Check if we have all needed reveals
                let reveals_needed = match deal_type {
                    DealType::HoleCard { .. } => self.game.num_players() - 1,
                    DealType::CommunityCard { .. } => self.game.num_players(),
                };

                if self.deal_reveals.len() >= reveals_needed {
                    let deal_type = deal_type.clone();
                    let pos = *deck_position;
                    self.finish_dealing_card(&deal_type, pos);
                }
                Ok(())
            }

            // --- Betting ---
            (Phase::Betting, Action::Bet { player_id, action }) => {
                if self.game.action_on != Some(*player_id) {
                    return Err(crate::Error::InvalidAction("not your turn to bet".into()));
                }

                let round_over = self.game.apply_bet(*player_id, action);

                if self.game.active_player_count() <= 1 {
                    self.phase = Phase::Complete;
                    return Ok(());
                }

                if round_over {
                    self.skip_to_next_deal();
                }
                Ok(())
            }

            // --- Showdown ---
            (Phase::Showdown, Action::RevealHand { player_id, scalars }) => {
                if self.game.players[*player_id].folded {
                    return Err(crate::Error::InvalidAction("folded players don't reveal".into()));
                }
                if self.showdown_revealed[*player_id] {
                    return Err(crate::Error::InvalidAction("already revealed".into()));
                }

                // Apply this player's lock keys to their hole cards to fully decrypt
                for (pos, scalar) in scalars {
                    // Find which hole card index this position is
                    if let Some(idx) = self.hole_card_positions[*player_id]
                        .iter()
                        .position(|p| p == pos)
                    {
                        if idx < self.game.players[*player_id].hole_encrypted.len() {
                            let decrypted = crypto::decrypt(
                                &self.game.players[*player_id].hole_encrypted[idx],
                                scalar,
                            )?;
                            self.game.players[*player_id].hole_points.push(decrypted);
                        }
                    }
                }

                self.showdown_revealed[*player_id] = true;

                let all_revealed = self
                    .game
                    .players
                    .iter()
                    .enumerate()
                    .all(|(i, p)| p.folded || self.showdown_revealed[i]);

                if all_revealed {
                    self.game.street = Street::Showdown;
                    self.phase = Phase::Complete;
                }
                Ok(())
            }

            _ => Err(crate::Error::InvalidAction(format!(
                "action {:?} not valid in phase {:?}",
                std::mem::discriminant(action),
                self.phase
            ))),
        }
    }

    pub fn valid_actions(&self) -> Vec<ValidAction> {
        match &self.phase {
            Phase::WaitingForPlayers { need } => (0..self.game.num_players())
                .filter(|_| *need > 0)
                .map(|pid| ValidAction {
                    player_id: pid,
                    kind: ValidActionKind::Join,
                })
                .collect(),
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
            Phase::Shuffle { next_player } => vec![ValidAction {
                player_id: *next_player,
                kind: ValidActionKind::ShuffleDeck,
            }],
            Phase::Lock { next_player } => vec![ValidAction {
                player_id: *next_player,
                kind: ValidActionKind::LockDeck,
            }],
            Phase::Dealing {
                deal_type,
                deck_position,
            } => {
                let exclude = match deal_type {
                    DealType::HoleCard { for_player, .. } => Some(*for_player),
                    DealType::CommunityCard { .. } => None,
                };
                (0..self.game.num_players())
                    .filter(|pid| exclude != Some(*pid) && !self.deal_reveals.contains_key(pid))
                    .map(|pid| ValidAction {
                        player_id: pid,
                        kind: ValidActionKind::RevealLockKey {
                            deck_position: *deck_position,
                        },
                    })
                    .collect()
            }
            Phase::Betting => {
                if let Some(pid) = self.game.action_on {
                    vec![ValidAction {
                        player_id: pid,
                        kind: ValidActionKind::Bet {
                            options: self.valid_bet_actions(pid),
                        },
                    }]
                } else {
                    vec![]
                }
            }
            Phase::Showdown => self
                .game
                .players
                .iter()
                .enumerate()
                .filter(|(i, p)| !p.folded && !self.showdown_revealed[*i])
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
        let to_call = self
            .game
            .current_bet
            .saturating_sub(player.bet_this_street);
        let mut actions = Vec::new();

        if to_call == 0 {
            actions.push(BetAction::Check);
        } else if player.chips >= to_call {
            actions.push(BetAction::Call);
        }

        actions.push(BetAction::Fold);

        let min_raise = self.game.big_blind;
        if player.chips > to_call + min_raise {
            actions.push(BetAction::Raise(to_call + min_raise));
        }

        if player.chips > 0 {
            actions.push(BetAction::AllIn);
        }

        actions
    }

    fn start_dealing_hole_cards(&mut self) {
        let n = self.game.num_players();
        let first_seat = (self.game.button + 1) % n;

        self.hole_deal_queue.clear();
        for card_idx in 0..2 {
            for offset in 0..n {
                let pid = (first_seat + offset) % n;
                self.hole_deal_queue.push((pid, card_idx));
            }
        }

        self.next_deck_position = 0;
        self.start_next_deal();
    }

    fn start_next_deal(&mut self) {
        if let Some((for_player, card_idx)) = self.hole_deal_queue.first().cloned() {
            let pos = self.next_deck_position;
            self.deal_reveals.clear();
            self.hole_card_positions[for_player].push(pos);
            self.phase = Phase::Dealing {
                deal_type: DealType::HoleCard { for_player, card_idx },
                deck_position: pos,
            };
        }
    }

    /// Called when all required lock keys have been revealed for a card.
    fn finish_dealing_card(&mut self, deal_type: &DealType, deck_position: usize) {
        // Apply all revealed lock scalars to the deck point to get partially-decrypted point
        let mut point = self.game.deck[deck_position].clone();
        for scalar in self.deal_reveals.values() {
            point = crypto::decrypt(&point, scalar).unwrap();
        }

        match deal_type {
            DealType::HoleCard { for_player, .. } => {
                // Point still has the recipient's lock key on it — they decrypt locally
                self.game.players[*for_player]
                    .hole_encrypted
                    .push(point);
                self.next_deck_position += 1;
                self.deal_reveals.clear();
                self.hole_deal_queue.remove(0);

                if self.hole_deal_queue.is_empty() {
                    self.start_betting_or_skip();
                } else {
                    self.start_next_deal();
                }
            }
            DealType::CommunityCard {
                remaining_this_street,
            } => {
                // All players revealed — point is fully decrypted
                self.game.community.push(point);
                self.next_deck_position += 1;
                self.deal_reveals.clear();
                let remaining = remaining_this_street - 1;

                if remaining == 0 {
                    self.start_betting_or_skip();
                } else {
                    let pos = self.next_deck_position;
                    self.phase = Phase::Dealing {
                        deal_type: DealType::CommunityCard {
                            remaining_this_street: remaining,
                        },
                        deck_position: pos,
                    };
                }
            }
        }
    }

    fn start_betting_or_skip(&mut self) {
        self.game.start_betting_round();
        if self.game.actionable_player_count() <= 1 {
            self.skip_to_next_deal();
        } else {
            self.phase = Phase::Betting;
        }
    }

    fn skip_to_next_deal(&mut self) {
        match self.game.street {
            Street::Preflop => {
                self.game.new_street(Street::Flop);
                self.start_community_deal(3);
            }
            Street::Flop => {
                self.game.new_street(Street::Turn);
                self.start_community_deal(1);
            }
            Street::Turn => {
                self.game.new_street(Street::River);
                self.start_community_deal(1);
            }
            Street::River => {
                self.phase = Phase::Showdown;
                self.showdown_revealed = vec![false; self.game.num_players()];
            }
            Street::Showdown => unreachable!(),
        }
    }

    fn start_community_deal(&mut self, num_cards: usize) {
        let pos = self.next_deck_position;
        self.deal_reveals.clear();
        self.phase = Phase::Dealing {
            deal_type: DealType::CommunityCard {
                remaining_this_street: num_cards,
            },
            deck_position: pos,
        };
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

    #[test]
    fn join_transitions_to_commit() {
        let mut state = ProtocolState::new(2, 1000, 10);
        state.apply(&Action::Join { player_id: 0 }).unwrap();
        assert!(matches!(state.phase, Phase::WaitingForPlayers { need: 1 }));
        state.apply(&Action::Join { player_id: 1 }).unwrap();
        assert!(matches!(state.phase, Phase::CommitSeeds));
    }

    #[test]
    fn seed_commit_reveal_flow() {
        let mut state = ProtocolState::new(2, 1000, 10);
        state.apply(&Action::Join { player_id: 0 }).unwrap();
        state.apply(&Action::Join { player_id: 1 }).unwrap();

        let seed0 = b"player0_secret_seed".to_vec();
        let seed1 = b"player1_secret_seed".to_vec();
        let hash0 = crypto::blake2b(&seed0).unwrap();
        let hash1 = crypto::blake2b(&seed1).unwrap();

        state
            .apply(&Action::CommitSeed {
                player_id: 0,
                commitment: hash0,
            })
            .unwrap();
        state
            .apply(&Action::CommitSeed {
                player_id: 1,
                commitment: hash1,
            })
            .unwrap();
        assert!(matches!(state.phase, Phase::RevealSeeds));

        state
            .apply(&Action::RevealSeed {
                player_id: 0,
                seed: seed0,
            })
            .unwrap();
        state
            .apply(&Action::RevealSeed {
                player_id: 1,
                seed: seed1,
            })
            .unwrap();
        assert!(matches!(state.phase, Phase::Shuffle { next_player: 0 }));
        assert!(state.combined_seed.is_some());
        assert_eq!(state.game.deck.len(), 52);
    }

    #[test]
    fn bad_seed_reveal_rejected() {
        let mut state = ProtocolState::new(2, 1000, 10);
        state.apply(&Action::Join { player_id: 0 }).unwrap();
        state.apply(&Action::Join { player_id: 1 }).unwrap();

        let seed0 = b"real_seed".to_vec();
        let hash0 = crypto::blake2b(&seed0).unwrap();
        state
            .apply(&Action::CommitSeed {
                player_id: 0,
                commitment: hash0,
            })
            .unwrap();
        let hash1 = crypto::blake2b(b"seed1").unwrap();
        state
            .apply(&Action::CommitSeed {
                player_id: 1,
                commitment: hash1,
            })
            .unwrap();

        let result = state.apply(&Action::RevealSeed {
            player_id: 0,
            seed: b"fake_seed".to_vec(),
        });
        assert!(result.is_err());
    }
}
