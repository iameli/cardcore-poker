//! Mental poker protocol state machine.
//!
//! The protocol progresses through phases, each requiring specific actions
//! from specific players. `valid_actions()` returns what's currently expected,
//! making it easy to fuzz test by randomly picking valid actions.

use serde::{Deserialize, Serialize};

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
    /// Sequential decryption for dealing a card.
    /// Each player (except the recipient for hole cards) decrypts in turn.
    Dealing {
        /// What kind of card is being dealt.
        deal_type: DealType,
        /// Deck position of the card being dealt.
        deck_position: usize,
        /// The current partially-decrypted point.
        current_point: Point,
        /// Which player should decrypt next.
        next_decryptor: PlayerId,
        /// How many players have decrypted so far.
        decryptions_done: usize,
        /// How many decryptions are needed.
        decryptions_needed: usize,
    },
    /// Betting round.
    Betting,
    /// Players reveal hole cards for showdown.
    Showdown,
    /// Hand is complete.
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DealType {
    /// Dealing hole card `card_idx` (0 or 1) to `for_player`.
    HoleCard {
        for_player: PlayerId,
        card_idx: usize,
    },
    /// Dealing a community card.
    CommunityCard {
        /// Total community cards to deal in this street (3 for flop, 1 for turn/river).
        remaining_this_street: usize,
    },
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
    /// Provide the result of decrypting a card (sends the decrypted point, not the key).
    DecryptCard {
        player_id: PlayerId,
        deck_position: usize,
        result: Point,
    },
    /// Betting action.
    Bet {
        player_id: PlayerId,
        action: BetAction,
    },
    /// Reveal hole cards at showdown (reveal decryption scalar for verification).
    RevealHand {
        player_id: PlayerId,
        scalar: Scalar,
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
    /// Decrypt card at this deck position.
    DecryptCard { deck_position: usize },
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
    /// Track which players have revealed their hands at showdown.
    pub showdown_revealed: Vec<bool>,
    /// Next deck position to deal from.
    pub next_deck_position: usize,
    /// Track hole card dealing progress: (player, card_idx) for next hole card.
    hole_deal_queue: Vec<(PlayerId, usize)>,
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
            showdown_revealed: vec![false; num_players],
            next_deck_position: 0,
            hole_deal_queue: Vec::new(),
        }
    }

    /// Apply an action to advance the protocol state.
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
                    self.game.post_blinds();
                    self.start_dealing_hole_cards();
                } else {
                    self.phase = Phase::Shuffle {
                        next_player: self.shuffles_done,
                    };
                }
                Ok(())
            }

            // --- Decrypt Card ---
            (
                Phase::Dealing {
                    deal_type,
                    deck_position,
                    current_point: _,
                    next_decryptor,
                    decryptions_done,
                    decryptions_needed,
                },
                Action::DecryptCard {
                    player_id,
                    deck_position: action_pos,
                    result,
                },
            ) => {
                if *player_id != *next_decryptor {
                    return Err(crate::Error::InvalidAction(
                        "not your turn to decrypt".into(),
                    ));
                }
                if *action_pos != *deck_position {
                    return Err(crate::Error::InvalidAction("wrong deck position".into()));
                }

                // Accept the decrypted point (we trust it for now; ZKP verification later)
                let new_done = decryptions_done + 1;
                let needed = *decryptions_needed;
                let deal_type = deal_type.clone();
                let pos = *deck_position;

                if new_done >= needed {
                    // Card is fully decrypted (by all required players)
                    self.finish_dealing_card(&deal_type, result);
                } else {
                    // Find next decryptor
                    let next = self.next_decryptor_after(*player_id, &deal_type);
                    self.phase = Phase::Dealing {
                        deal_type,
                        deck_position: pos,
                        current_point: result.clone(),
                        next_decryptor: next,
                        decryptions_done: new_done,
                        decryptions_needed: needed,
                    };
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
            (Phase::Showdown, Action::RevealHand { player_id, scalar }) => {
                if self.game.players[*player_id].folded {
                    return Err(crate::Error::InvalidAction("folded players don't reveal".into()));
                }
                if self.showdown_revealed[*player_id] {
                    return Err(crate::Error::InvalidAction("already revealed".into()));
                }

                // Decrypt this player's hole cards with their revealed key
                for i in 0..self.game.players[*player_id].hole_encrypted.len() {
                    let decrypted = crypto::decrypt(
                        &self.game.players[*player_id].hole_encrypted[i],
                        scalar,
                    )?;
                    self.game.players[*player_id].hole_points.push(decrypted);
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

    /// Returns all actions that are valid in the current state.
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
            Phase::Dealing {
                next_decryptor,
                deck_position,
                ..
            } => vec![ValidAction {
                player_id: *next_decryptor,
                kind: ValidActionKind::DecryptCard {
                    deck_position: *deck_position,
                },
            }],
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

    /// Build the hole card dealing queue: round-robin, left of button, 2 rounds.
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

    /// Start dealing the next card (hole or community).
    fn start_next_deal(&mut self) {
        if let Some((for_player, card_idx)) = self.hole_deal_queue.first().cloned() {
            let pos = self.next_deck_position;
            let n = self.game.num_players();
            // For hole cards, everyone EXCEPT the recipient decrypts
            let decryptions_needed = n - 1;
            let first_decryptor = self.first_decryptor_excluding(Some(for_player));
            self.phase = Phase::Dealing {
                deal_type: DealType::HoleCard { for_player, card_idx },
                deck_position: pos,
                current_point: self.game.deck[pos].clone(),
                next_decryptor: first_decryptor,
                decryptions_done: 0,
                decryptions_needed,
            };
        }
        // If queue is empty, caller handles transition
    }

    /// Called when a card has been fully decrypted by all required players.
    fn finish_dealing_card(&mut self, deal_type: &DealType, final_point: &Point) {
        match deal_type {
            DealType::HoleCard { for_player, .. } => {
                // Store the partially-decrypted point — recipient still needs to apply their key
                self.game.players[*for_player]
                    .hole_encrypted
                    .push(final_point.clone());
                self.next_deck_position += 1;
                self.hole_deal_queue.remove(0);

                if self.hole_deal_queue.is_empty() {
                    // All hole cards dealt
                    self.start_betting_or_skip();
                } else {
                    self.start_next_deal();
                }
            }
            DealType::CommunityCard {
                remaining_this_street,
            } => {
                self.game.community.push(final_point.clone());
                self.next_deck_position += 1;
                let remaining = remaining_this_street - 1;

                if remaining == 0 {
                    // Done dealing community cards for this street
                    self.start_betting_or_skip();
                } else {
                    // Deal next community card
                    let pos = self.next_deck_position;
                    let n = self.game.num_players();
                    self.phase = Phase::Dealing {
                        deal_type: DealType::CommunityCard {
                            remaining_this_street: remaining,
                        },
                        deck_position: pos,
                        current_point: self.game.deck[pos].clone(),
                        next_decryptor: 0,
                        decryptions_done: 0,
                        decryptions_needed: n,
                    };
                }
            }
        }
    }

    /// Find the first player who should decrypt, optionally excluding one player.
    fn first_decryptor_excluding(&self, exclude: Option<PlayerId>) -> PlayerId {
        for i in 0..self.game.num_players() {
            if exclude != Some(i) {
                return i;
            }
        }
        0
    }

    /// Find the next player who should decrypt after `current`, respecting exclusions.
    fn next_decryptor_after(&self, current: PlayerId, deal_type: &DealType) -> PlayerId {
        let exclude = match deal_type {
            DealType::HoleCard { for_player, .. } => Some(*for_player),
            DealType::CommunityCard { .. } => None,
        };
        let n = self.game.num_players();
        for offset in 1..n {
            let pid = (current + offset) % n;
            if exclude != Some(pid) {
                return pid;
            }
        }
        current
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
        let n = self.game.num_players();
        self.phase = Phase::Dealing {
            deal_type: DealType::CommunityCard {
                remaining_this_street: num_cards,
            },
            deck_position: pos,
            current_point: self.game.deck[pos].clone(),
            next_decryptor: 0,
            decryptions_done: 0,
            decryptions_needed: n,
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
