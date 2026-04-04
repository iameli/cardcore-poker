//! Mental poker protocol state machine.
//!
//! The protocol progresses through phases, each requiring specific actions
//! from specific players. `valid_actions()` returns what's currently expected,
//! making it easy to fuzz test by randomly picking valid actions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::crypto::{self, Point, Scalar};
use crate::game::{BetAction, GameState, PlayerId, Street};

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
    DealHole {
        for_player: PlayerId,
        card_idx: usize,
    },
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
    /// Track which players have provided decryption shares for current deal.
    /// Maps player_id -> true for those who have submitted their share.
    pub decrypt_shares: HashMap<PlayerId, bool>,
    /// The current partially-decrypted point being dealt.
    pub dealing_point: Option<Point>,
    /// How many community cards have been dealt so far in the current DealCommunity phase.
    pub community_dealt_this_phase: usize,
    /// Track which players have revealed their hands at showdown.
    pub showdown_revealed: Vec<bool>,
    /// Total number of cards dealt from the deck so far (for deck position tracking).
    pub cards_dealt: usize,
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
            decrypt_shares: HashMap::new(),
            dealing_point: None,
            community_dealt_this_phase: 0,
            showdown_revealed: vec![false; num_players],
            cards_dealt: 0,
        }
    }

    /// Apply an action to advance the protocol state.
    /// Returns an error if the action is invalid for the current state.
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
            (Phase::CommitSeeds, Action::CommitSeed { player_id, commitment }) => {
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
                // Verify commitment
                let hash = crypto::blake2b(seed)?;
                let commitment = self.seed_commitments[*player_id]
                    .ok_or_else(|| crate::Error::InvalidAction("no commitment found".into()))?;
                if hash != commitment {
                    return Err(crate::Error::Crypto("seed doesn't match commitment".into()));
                }
                self.seeds_revealed[*player_id] = Some(seed.clone());

                if self.seeds_revealed.iter().all(|s| s.is_some()) {
                    // Combine all seeds
                    let mut combined = Vec::new();
                    for s in &self.seeds_revealed {
                        combined.extend_from_slice(s.as_ref().unwrap());
                    }
                    self.combined_seed = Some(crypto::blake2b(&combined)?);

                    // Initialize plaintext deck for first player to shuffle
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
                    // All players have shuffled — post blinds and start dealing
                    self.game.post_blinds();
                    self.start_dealing_hole_cards();
                } else {
                    self.phase = Phase::Shuffle {
                        next_player: self.shuffles_done,
                    };
                }
                Ok(())
            }

            // --- Decrypt Card (for dealing) ---
            (
                Phase::DealHole { for_player, card_idx },
                Action::DecryptCard {
                    player_id,
                    position,
                    scalar,
                },
            ) => {
                let for_player = *for_player;
                let card_idx = *card_idx;

                if *player_id == for_player {
                    return Err(crate::Error::InvalidAction(
                        "recipient doesn't decrypt their own card during deal".into(),
                    ));
                }
                if *position != self.deck_position_for_hole(for_player, card_idx) {
                    return Err(crate::Error::InvalidAction("wrong card position".into()));
                }
                if self.decrypt_shares.contains_key(player_id) {
                    return Err(crate::Error::InvalidAction("already provided share".into()));
                }

                // Apply this player's decryption to the partially-decrypted point
                let current = self
                    .dealing_point
                    .as_ref()
                    .unwrap_or(&self.game.deck[*position]);
                let decrypted = crypto::decrypt(current, scalar)?;
                self.dealing_point = Some(decrypted.clone());
                self.decrypt_shares.insert(*player_id, true);

                // Check if all other players have provided shares
                let shares_needed = self.game.num_players() - 1;
                if self.decrypt_shares.len() >= shares_needed {
                    // The recipient's own decryption key still needs to be applied,
                    // but they do that locally. Store the partially-decrypted point.
                    self.game.players[for_player]
                        .hole_encrypted
                        .push(decrypted);

                    // Move to next hole card
                    self.decrypt_shares.clear();
                    self.dealing_point = None;
                    self.advance_hole_dealing(for_player, card_idx);
                }
                Ok(())
            }

            // --- Decrypt Card (for community) ---
            (
                Phase::DealCommunity { num_to_deal },
                Action::DecryptCard {
                    player_id,
                    position,
                    scalar,
                },
            ) => {
                let num_to_deal = *num_to_deal;
                let expected_pos = self.next_community_deck_position();

                if *position != expected_pos {
                    return Err(crate::Error::InvalidAction("wrong card position".into()));
                }
                if self.decrypt_shares.contains_key(player_id) {
                    return Err(crate::Error::InvalidAction("already provided share".into()));
                }

                let current = self
                    .dealing_point
                    .as_ref()
                    .unwrap_or(&self.game.deck[*position]);
                let decrypted = crypto::decrypt(current, scalar)?;
                self.dealing_point = Some(decrypted.clone());
                self.decrypt_shares.insert(*player_id, true);

                if self.decrypt_shares.len() >= self.game.num_players() {
                    // All players decrypted — this is a community card everyone can see
                    self.game.community.push(decrypted);
                    self.cards_dealt += 1;
                    self.community_dealt_this_phase += 1;
                    self.decrypt_shares.clear();
                    self.dealing_point = None;

                    if self.community_dealt_this_phase >= num_to_deal {
                        // Done dealing community cards for this street
                        self.community_dealt_this_phase = 0;
                        self.advance_street();
                    }
                    // Otherwise stay in DealCommunity, next card will be dealt
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
                    // Everyone folded (or one left) — hand is over
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
                if scalars.len() != 2 {
                    return Err(crate::Error::InvalidAction(
                        "must reveal exactly 2 hole card scalars".into(),
                    ));
                }

                // Apply the player's own decryption keys to their hole cards
                // to get the final plaintext points
                for (i, scalar) in scalars.iter().enumerate() {
                    if i < self.game.players[*player_id].hole_encrypted.len() {
                        let decrypted = crypto::decrypt(
                            &self.game.players[*player_id].hole_encrypted[i],
                            scalar,
                        )?;
                        self.game.players[*player_id].hole_points.push(decrypted);
                    }
                }

                self.showdown_revealed[*player_id] = true;

                // Check if all non-folded players have revealed
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

    /// Returns all actions that are valid in the current state.
    /// This is the key function for fuzz testing — randomly pick from this list.
    pub fn valid_actions(&self) -> Vec<ValidAction> {
        match &self.phase {
            Phase::WaitingForPlayers { need } => {
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
            Phase::DealHole {
                for_player,
                card_idx,
            } => {
                let pos = self.deck_position_for_hole(*for_player, *card_idx);
                (0..self.game.num_players())
                    .filter(|pid| *pid != *for_player && !self.decrypt_shares.contains_key(pid))
                    .map(|pid| ValidAction {
                        player_id: pid,
                        kind: ValidActionKind::DecryptCard { position: pos },
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
            Phase::DealCommunity { .. } => {
                let next_pos = self.next_community_deck_position();
                (0..self.game.num_players())
                    .filter(|pid| !self.decrypt_shares.contains_key(pid))
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

    /// Deck position for a player's hole card.
    /// Cards are dealt round-robin starting left of button:
    /// player0_card0, player1_card0, ..., player0_card1, player1_card1, ...
    pub fn deck_position_for_hole_public(&self, for_player: PlayerId, card_idx: usize) -> usize {
        self.deck_position_for_hole(for_player, card_idx)
    }

    fn deck_position_for_hole(&self, for_player: PlayerId, card_idx: usize) -> usize {
        let n = self.game.num_players();
        // Deal order starts left of button
        let first_seat = (self.game.button + 1) % n;
        let player_offset = if for_player >= first_seat {
            for_player - first_seat
        } else {
            n - first_seat + for_player
        };
        card_idx * n + player_offset
    }

    /// Which deck position is the next community card?
    fn next_community_deck_position(&self) -> usize {
        let hole_card_count = self.game.num_players() * 2;
        hole_card_count + self.game.community.len()
    }

    /// Start dealing hole cards: 2 per player, round-robin from left of button.
    fn start_dealing_hole_cards(&mut self) {
        let first_seat = (self.game.button + 1) % self.game.num_players();
        self.decrypt_shares.clear();
        self.dealing_point = None;
        self.phase = Phase::DealHole {
            for_player: first_seat,
            card_idx: 0,
        };
    }

    /// Advance to the next hole card to deal, or transition to betting.
    fn advance_hole_dealing(&mut self, current_player: PlayerId, current_card_idx: usize) {
        let n = self.game.num_players();
        let first_seat = (self.game.button + 1) % n;

        // Next player in deal order
        let next_player = (current_player + 1) % n;
        let wrapped = next_player == first_seat;

        if wrapped {
            // Finished this round of cards
            let next_card_idx = current_card_idx + 1;
            if next_card_idx >= 2 {
                // All hole cards dealt — start preflop betting
                self.cards_dealt = n * 2;
                self.start_betting_or_skip();
            } else {
                self.phase = Phase::DealHole {
                    for_player: first_seat,
                    card_idx: next_card_idx,
                };
            }
        } else {
            self.phase = Phase::DealHole {
                for_player: next_player,
                card_idx: current_card_idx,
            };
        }
    }

    /// After a betting round ends, advance to the next street.
    fn advance_street(&mut self) {
        if self.game.active_player_count() <= 1 {
            self.phase = Phase::Complete;
            return;
        }
        self.start_betting_or_skip();
    }

    /// Transition to betting phase, or skip it if no one can act
    /// (e.g., everyone is all-in).
    fn start_betting_or_skip(&mut self) {
        self.game.start_betting_round();
        if self.game.actionable_player_count() <= 1 {
            // No meaningful betting possible — advance to next deal or showdown
            self.skip_to_next_deal();
        } else {
            self.phase = Phase::Betting;
        }
    }

    /// Skip directly to the next community deal or showdown when betting can't happen.
    fn skip_to_next_deal(&mut self) {
        match self.game.street {
            Street::Preflop => {
                self.game.new_street(Street::Flop);
                self.phase = Phase::DealCommunity { num_to_deal: 3 };
            }
            Street::Flop => {
                self.game.new_street(Street::Turn);
                self.phase = Phase::DealCommunity { num_to_deal: 1 };
            }
            Street::Turn => {
                self.game.new_street(Street::River);
                self.phase = Phase::DealCommunity { num_to_deal: 1 };
            }
            Street::River => {
                self.phase = Phase::Showdown;
                self.showdown_revealed = vec![false; self.game.num_players()];
            }
            Street::Showdown => unreachable!(),
        }
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

        // Commit seeds
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

        // Reveal seeds
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

        // Try to reveal a different seed
        let result = state.apply(&Action::RevealSeed {
            player_id: 0,
            seed: b"fake_seed".to_vec(),
        });
        assert!(result.is_err());
    }
}
