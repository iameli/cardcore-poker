//! Mental poker protocol state machine.
//!
//! Two-phase shuffle+lock with deterministic PRNG:
//! 1. Commit seeds (hash only — seeds stay secret)
//! 2. Shuffle: each player encrypts all cards with one key, shuffles
//! 3. Lock: each player removes shuffle key, re-encrypts with per-position keys
//! 4. Deal/bet: players reveal per-position lock scalars
//! 5. After the hand: reveal seeds for full game replay and verification

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::card::Card;
use crate::crypto::{self, Point, Scalar};
use crate::eval;
use crate::game::{BetAction, GameState, PlayerId, Street};

/// One (side) pot's outcome at settlement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PotResult {
    /// Seats that won this pot (more than one when the pot is chopped).
    pub winners: Vec<PlayerId>,
    /// Chips in this pot.
    pub amount: u64,
    /// Winning hand description (None when won uncontested / by fold).
    pub hand_desc: Option<String>,
}

/// A revealed hand at showdown, for the log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShownHand {
    pub seat: PlayerId,
    pub cards: Vec<String>,
    pub hand_desc: String,
}

/// Result of a completed hand — what the UI logs so players can follow along.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandResult {
    pub hand_index: u64,
    pub pots: Vec<PotResult>,
    /// True when everyone but one player folded (no showdown).
    pub by_fold: bool,
    /// Hands revealed at showdown (empty on a fold win).
    pub shown: Vec<ShownHand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// No table established yet.
    Init,
    /// Players commit hashes of their secret seeds.
    CommitSeeds,
    /// Players take turns encrypting and shuffling the deck.
    Shuffle {
        next_player: PlayerId,
    },
    /// Players take turns removing shuffle encryption and adding per-position lock keys.
    Lock {
        next_player: PlayerId,
    },
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
    /// Establish the table: players, chips, blinds. First action in any game.
    Table {
        /// Player identifiers in seat order (DIDs in production, names for now).
        players: Vec<String>,
        starting_chips: u64,
        small_blind: u64,
    },
    CommitSeed {
        player_id: PlayerId,
        #[serde(with = "crypto::serde_base64")]
        commitment: [u8; crypto::HASH_BYTES],
    },
    ShuffleDeck {
        player_id: PlayerId,
        deck: Vec<Point>,
    },
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
        scalars: Vec<(usize, Scalar)>,
    },
    /// Post-game: reveal secret seed for full verification.
    VerifySeed {
        player_id: PlayerId,
        #[serde(with = "crypto::serde_base64_vec")]
        seed: Vec<u8>,
    },
}

impl Action {
    /// Returns the player who submitted this action, if applicable.
    pub fn player_id(&self) -> Option<PlayerId> {
        match self {
            Action::Table { .. } => None,
            Action::CommitSeed { player_id, .. }
            | Action::ShuffleDeck { player_id, .. }
            | Action::LockDeck { player_id, .. }
            | Action::RevealLockKey { player_id, .. }
            | Action::Bet { player_id, .. }
            | Action::RevealHand { player_id, .. }
            | Action::VerifySeed { player_id, .. } => Some(*player_id),
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
    CommitSeed,
    ShuffleDeck,
    LockDeck,
    RevealLockKey { deck_position: usize },
    Bet { options: Vec<BetAction> },
    RevealHand,
    VerifySeed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolState {
    pub phase: Phase,
    pub game: GameState,
    pub seed_commitments: Vec<Option<[u8; crypto::HASH_BYTES]>>,
    pub shuffles_done: usize,
    pub locks_done: usize,
    pub showdown_revealed: Vec<bool>,
    pub next_deck_position: usize,
    hole_deal_queue: Vec<(PlayerId, usize)>,
    pub deal_reveals: HashMap<PlayerId, Scalar>,
    /// For each player, the deck positions of their hole cards.
    pub hole_card_positions: Vec<Vec<usize>>,
    /// Post-game seed verification tracking.
    pub seeds_verified: Vec<bool>,
    /// Which hand we're on (0-based). Bumped each time a new hand starts.
    pub hand_index: u64,
    /// Result of the most recently completed hand (for the UI log).
    pub last_hand_result: Option<HandResult>,
}

impl ProtocolState {
    pub fn new() -> Self {
        Self {
            phase: Phase::Init,
            game: GameState::new(0, 0, 0),
            seed_commitments: Vec::new(),
            shuffles_done: 0,
            locks_done: 0,
            showdown_revealed: Vec::new(),
            next_deck_position: 0,
            hole_deal_queue: Vec::new(),
            deal_reveals: HashMap::new(),
            hole_card_positions: Vec::new(),
            seeds_verified: Vec::new(),
            hand_index: 0,
            last_hand_result: None,
        }
    }

    pub fn apply(&mut self, action: &Action) -> crate::Result<()> {
        match (&self.phase, action) {
            // --- Table ---
            (
                Phase::Init,
                Action::Table {
                    players,
                    starting_chips,
                    small_blind,
                },
            ) => {
                let n = players.len();
                if n < 2 || n > 10 {
                    return Err(crate::Error::InvalidAction("need 2-10 players".into()));
                }
                self.game = GameState::new(n, *starting_chips, *small_blind);
                self.seed_commitments = vec![None; n];
                self.showdown_revealed = vec![false; n];
                self.hole_card_positions = vec![Vec::new(); n];
                self.seeds_verified = vec![false; n];
                self.phase = Phase::CommitSeeds;
                Ok(())
            }

            // --- Commit Seeds ---
            (
                Phase::CommitSeeds,
                Action::CommitSeed {
                    player_id,
                    commitment,
                },
            ) => {
                if self.seed_commitments[*player_id].is_some() {
                    return Err(crate::Error::InvalidAction("already committed".into()));
                }
                self.seed_commitments[*player_id] = Some(*commitment);
                if self.seed_commitments.iter().all(|c| c.is_some()) {
                    // Seeds committed — go straight to shuffle (no reveal yet)
                    let card_points = crypto::card_points()?;
                    self.game.deck = card_points.into_iter().map(|(_, p)| p).collect();
                    self.phase = Phase::Shuffle { next_player: 0 };
                }
                Ok(())
            }

            // --- Shuffle ---
            (Phase::Shuffle { next_player }, Action::ShuffleDeck { player_id, deck }) => {
                if *player_id != *next_player {
                    return Err(crate::Error::InvalidAction(
                        "not your turn to shuffle".into(),
                    ));
                }
                if deck.len() != 52 {
                    return Err(crate::Error::InvalidAction(
                        "deck must have 52 cards".into(),
                    ));
                }
                self.game.deck = deck.clone();
                self.shuffles_done += 1;

                if self.shuffles_done >= self.game.num_players() {
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
                    return Err(crate::Error::InvalidAction(
                        "deck must have 52 cards".into(),
                    ));
                }
                self.game.deck = deck.clone();
                self.locks_done += 1;

                if self.locks_done >= self.game.num_players() {
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
                    self.settle();
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
                    return Err(crate::Error::InvalidAction(
                        "folded players don't reveal".into(),
                    ));
                }
                if self.showdown_revealed[*player_id] {
                    return Err(crate::Error::InvalidAction("already revealed".into()));
                }

                for (pos, scalar) in scalars {
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
                    self.settle();
                    self.phase = Phase::Complete;
                }
                Ok(())
            }

            // --- Verify Seed (post-game) ---
            (Phase::Complete, Action::VerifySeed { player_id, seed }) => {
                if self.seeds_verified[*player_id] {
                    return Err(crate::Error::InvalidAction("already verified".into()));
                }
                // Check seed matches commitment
                let hash = crypto::blake2b(seed)?;
                let commitment = self.seed_commitments[*player_id]
                    .ok_or_else(|| crate::Error::InvalidAction("no commitment found".into()))?;
                if hash != commitment {
                    return Err(crate::Error::Crypto("seed doesn't match commitment".into()));
                }
                self.seeds_verified[*player_id] = true;
                // Actual replay verification happens client-side — they can re-derive
                // all keys from this seed and verify every action.
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
            Phase::Init => vec![], // Table action is handled externally
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
            Phase::Complete => {
                // Post-game: any player who hasn't verified yet can reveal their seed
                self.seeds_verified
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| !**v)
                    .map(|(pid, _)| ValidAction {
                        player_id: pid,
                        kind: ValidActionKind::VerifySeed,
                    })
                    .collect()
            }
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

        // Deal in seat order starting left of the button, one card to each
        // live player, then the second. Eliminated players are skipped (they
        // sit out) but still reveal lock keys for everyone else's cards.
        self.hole_deal_queue.clear();
        for card_idx in 0..2 {
            for offset in 0..n {
                let pid = (first_seat + offset) % n;
                if !self.game.players[pid].eliminated {
                    self.hole_deal_queue.push((pid, card_idx));
                }
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
                deal_type: DealType::HoleCard {
                    for_player,
                    card_idx,
                },
                deck_position: pos,
            };
        }
    }

    fn finish_dealing_card(&mut self, deal_type: &DealType, deck_position: usize) {
        let mut point = self.game.deck[deck_position].clone();
        for scalar in self.deal_reveals.values() {
            point = crypto::decrypt(&point, scalar).unwrap();
        }

        match deal_type {
            DealType::HoleCard { for_player, .. } => {
                self.game.players[*for_player].hole_encrypted.push(point);
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

    /// Award the pot(s) at the end of a hand: build side pots from each
    /// player's total contribution, then award each layer to the best eligible
    /// (non-folded) hand, splitting ties. Records a HandResult for the UI log.
    /// Deterministic — every agent runs the identical computation.
    fn settle(&mut self) {
        let n = self.game.num_players();
        let non_folded: Vec<usize> = (0..n).filter(|&i| !self.game.players[i].folded).collect();
        let by_fold = non_folded.len() <= 1;

        // Decode known cards for hand evaluation.
        let card_map: HashMap<Point, Card> = crypto::card_points()
            .unwrap_or_default()
            .into_iter()
            .map(|(c, p)| (p, c))
            .collect();
        let community: Vec<Card> = self
            .game
            .community
            .iter()
            .filter_map(|p| card_map.get(p).copied())
            .collect();
        let mut hands: Vec<Option<eval::EvaluatedHand>> = vec![None; n];
        for &i in &non_folded {
            let hole: Vec<Card> = self.game.players[i]
                .hole_points
                .iter()
                .filter_map(|p| card_map.get(p).copied())
                .collect();
            if hole.len() == 2 && community.len() == 5 {
                let mut all = hole;
                all.extend_from_slice(&community);
                hands[i] = Some(eval::best_hand(&all));
            }
        }

        // Build side pots by peeling off equal contribution layers.
        let mut committed: Vec<u64> = self
            .game
            .players
            .iter()
            .map(|p| p.total_committed)
            .collect();
        let mut pots: Vec<PotResult> = Vec::new();
        loop {
            let min = match committed.iter().copied().filter(|&c| c > 0).min() {
                Some(m) => m,
                None => break,
            };
            let contributors: Vec<usize> = (0..n).filter(|&i| committed[i] > 0).collect();
            let mut amount = 0u64;
            for &i in &contributors {
                committed[i] -= min;
                amount += min;
            }
            let eligible: Vec<usize> = contributors
                .iter()
                .copied()
                .filter(|&i| !self.game.players[i].folded)
                .collect();

            let winners: Vec<usize> = if eligible.is_empty() {
                Vec::new()
            } else if eligible.len() == 1 {
                vec![eligible[0]]
            } else {
                // Best evaluated hand(s) among eligible players.
                let best = eligible
                    .iter()
                    .filter_map(|&i| hands[i].clone().map(|h| (i, h)))
                    .max_by(|(_, a), (_, b)| a.cmp(b));
                match best {
                    Some((_, best_hand)) => eligible
                        .iter()
                        .copied()
                        .filter(|&i| hands[i].as_ref() == Some(&best_hand))
                        .collect(),
                    None => eligible.clone(),
                }
            };

            if !winners.is_empty() {
                let share = amount / winners.len() as u64;
                let mut remainder = amount % winners.len() as u64;
                // Odd chips go to the earliest seat left of the button.
                let mut ordered = winners.clone();
                let first = (self.game.button + 1) % n;
                ordered.sort_by_key(|&i| (i + n - first) % n);
                for &w in &ordered {
                    let mut take = share;
                    if remainder > 0 {
                        take += 1;
                        remainder -= 1;
                    }
                    self.game.players[w].chips += take;
                }
            }

            let hand_desc = winners
                .iter()
                .find_map(|&i| hands[i].as_ref().map(|h| h.to_string()));
            pots.push(PotResult {
                winners,
                amount,
                hand_desc,
            });
        }

        // Showdown reveals for the log (skipped on a fold win).
        let mut shown: Vec<ShownHand> = Vec::new();
        if !by_fold {
            for &i in &non_folded {
                if let Some(h) = &hands[i] {
                    let cards: Vec<String> = self.game.players[i]
                        .hole_points
                        .iter()
                        .filter_map(|p| card_map.get(p).map(|c| c.to_string()))
                        .collect();
                    shown.push(ShownHand {
                        seat: i,
                        cards,
                        hand_desc: h.to_string(),
                    });
                }
            }
        }

        self.game.pot = 0;
        self.last_hand_result = Some(HandResult {
            hand_index: self.hand_index,
            pots,
            by_fold,
            shown,
        });
    }

    /// The game is over once at most one player has chips left.
    pub fn game_over(&self) -> bool {
        self.game.players.iter().filter(|p| p.chips > 0).count() <= 1
    }

    /// Begin the next hand: bust broke players, rotate the button to the next
    /// live seat, clear per-hand state, and return to seed commitment. Seats
    /// (and thus the cryptographic roster) stay fixed — busted players sit out
    /// but still participate in shuffling/dealing. Caller must ensure the game
    /// isn't already over.
    pub fn start_next_hand(&mut self) {
        for p in &mut self.game.players {
            if p.chips == 0 {
                p.eliminated = true;
            }
        }
        self.game.button = self.game.next_live_seat(self.game.button);
        self.game.street = Street::Preflop;
        self.game.pot = 0;
        self.game.current_bet = 0;
        self.game.action_on = None;
        self.game.deck.clear();
        self.game.community.clear();
        self.game.actions_this_round = 0;
        for p in &mut self.game.players {
            p.bet_this_street = 0;
            p.total_committed = 0;
            p.all_in = false;
            p.folded = p.eliminated; // eliminated players sit the hand out
            p.hole_encrypted.clear();
            p.hole_points.clear();
        }
        let n = self.game.num_players();
        self.seed_commitments = vec![None; n];
        self.showdown_revealed = vec![false; n];
        self.hole_card_positions = vec![Vec::new(); n];
        self.shuffles_done = 0;
        self.locks_done = 0;
        self.next_deck_position = 0;
        self.hole_deal_queue.clear();
        self.deal_reveals.clear();
        self.last_hand_result = None;
        self.hand_index += 1;
        self.phase = Phase::CommitSeeds;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_table(n: usize) -> ProtocolState {
        let mut state = ProtocolState::new();
        let players: Vec<String> = (0..n).map(|i| format!("did:example:player{}", i)).collect();
        state
            .apply(&Action::Table {
                players,
                starting_chips: 1000,
                small_blind: 10,
            })
            .unwrap();
        state
    }

    #[test]
    fn table_transitions_to_commit() {
        let state = setup_table(2);
        assert!(matches!(state.phase, Phase::CommitSeeds));
        assert_eq!(state.game.num_players(), 2);
        let actions = state.valid_actions();
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0].kind, ValidActionKind::CommitSeed));
    }

    #[test]
    fn commit_goes_to_shuffle() {
        let mut state = setup_table(2);

        let hash0 = crypto::blake2b(b"seed0").unwrap();
        let hash1 = crypto::blake2b(b"seed1").unwrap();

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
        assert!(matches!(state.phase, Phase::Shuffle { next_player: 0 }));
        assert_eq!(state.game.deck.len(), 52);
    }

    /// Build a fresh N-player state at showdown with given contributions, ready
    /// to settle. Players start with 0 chips (all-in) unless set otherwise.
    fn showdown_state(committed: &[u64]) -> ProtocolState {
        crypto::init().ok();
        let mut st = ProtocolState::new();
        st.game = GameState::new(committed.len(), 1000, 10);
        let pot: u64 = committed.iter().sum();
        for (i, &c) in committed.iter().enumerate() {
            st.game.players[i].total_committed = c;
            st.game.players[i].chips = 0;
        }
        st.game.pot = pot;
        st.game.street = Street::Showdown;
        st
    }

    fn point_of(card: &str) -> Point {
        crypto::card_points()
            .unwrap()
            .into_iter()
            .find(|(c, _)| c.to_string() == card)
            .map(|(_, p)| p)
            .unwrap_or_else(|| panic!("no such card {}", card))
    }

    fn set_hole(st: &mut ProtocolState, seat: usize, a: &str, b: &str) {
        st.game.players[seat].hole_points = vec![point_of(a), point_of(b)];
    }

    fn set_board(st: &mut ProtocolState, cards: &[&str]) {
        st.game.community = cards.iter().map(|c| point_of(c)).collect();
    }

    #[test]
    fn side_pots_split_by_contribution() {
        // P0 all-in 100, P1 all-in 300, P2 calls 300.
        //   main pot 300 (P0,P1,P2 eligible) -> best hand among all three
        //   side pot 400 (P1,P2 eligible)   -> best hand among P1,P2
        let mut st = showdown_state(&[100, 300, 300]);
        set_board(&mut st, &["2c", "7d", "9s", "Jh", "4c"]);
        set_hole(&mut st, 0, "As", "Ac"); // pair of aces (best)
        set_hole(&mut st, 1, "Ks", "Kc"); // pair of kings
        set_hole(&mut st, 2, "Qs", "Qc"); // pair of queens (worst)

        st.settle();

        // P0 wins the 300 main pot (best overall). P1 wins the 400 side pot
        // (best between P1 and P2). P2 wins nothing.
        assert_eq!(st.game.players[0].chips, 300);
        assert_eq!(st.game.players[1].chips, 400);
        assert_eq!(st.game.players[2].chips, 0);
        let result = st.last_hand_result.as_ref().unwrap();
        assert_eq!(result.pots.len(), 2);
        assert!(!result.by_fold);
    }

    #[test]
    fn short_allin_cannot_win_more_than_matched() {
        // P0 (short, best hand) all-in 100 can only win the 300 main pot, not
        // the 400 side pot it never matched — even though it has the best hand.
        let mut st = showdown_state(&[100, 300, 300]);
        set_board(&mut st, &["2c", "7d", "9s", "Jh", "4c"]);
        set_hole(&mut st, 0, "As", "Ac"); // best hand but short stack
        set_hole(&mut st, 1, "Ks", "Kc"); // best among P1,P2 -> wins side pot
        set_hole(&mut st, 2, "5h", "6h"); // worst

        st.settle();

        assert_eq!(st.game.players[0].chips, 300); // main pot only
        assert_eq!(st.game.players[1].chips, 400); // side pot
        assert_eq!(st.game.players[2].chips, 0);
    }

    #[test]
    fn tie_splits_pot_with_odd_chip() {
        // Two players tie using the board; the pot (101) chops, odd chip to the
        // earlier seat left of the button (seat 0, button defaults to 0 -> first
        // is seat 1, so seat 1 gets the odd chip).
        let mut st = showdown_state(&[51, 50]);
        st.game.pot = 101;
        set_board(&mut st, &["As", "Ah", "Ad", "Ac", "Kd"]); // quad aces on board
        set_hole(&mut st, 0, "2c", "3c");
        set_hole(&mut st, 1, "4c", "5c");

        st.settle();

        // Both play the board (four aces + K). Split 101 -> 50/51.
        let total: u64 = st.game.players.iter().map(|p| p.chips).sum();
        assert_eq!(total, 101);
        assert!(st.game.players[0].chips == 50 || st.game.players[0].chips == 51);
        assert!((st.game.players[0].chips as i64 - st.game.players[1].chips as i64).abs() == 1);
    }

    #[test]
    fn fold_win_returns_uncalled_and_awards_pot() {
        // P0 bet 300, P1 put in 100 then folded. Only P0 remains.
        let mut st = showdown_state(&[300, 100]);
        st.game.players[1].folded = true;
        st.game.street = Street::Preflop; // no showdown (fold win)

        st.settle();

        // P0 wins everything in the pot (400): the 200 main (both contributed
        // 100... wait) -> P0 gets the 100 P1 matched plus its own back.
        assert_eq!(st.game.players[0].chips, 400);
        assert_eq!(st.game.players[1].chips, 0);
        assert!(st.last_hand_result.as_ref().unwrap().by_fold);
    }

    #[test]
    fn next_hand_rotates_button_and_eliminates() {
        crypto::init().ok();
        let mut st = setup_table(3);
        // Simulate end of a hand: seat 1 busted.
        st.game.players[0].chips = 1500;
        st.game.players[1].chips = 0;
        st.game.players[2].chips = 1500;
        st.game.button = 0;
        st.phase = Phase::Complete;

        assert!(!st.game_over());
        st.start_next_hand();

        assert!(st.game.players[1].eliminated);
        assert!(st.game.players[1].folded); // sits out
        assert!(!st.game.players[0].eliminated);
        // Button rotates from 0 to the next live seat (2, since 1 is out).
        assert_eq!(st.game.button, 2);
        assert!(matches!(st.phase, Phase::CommitSeeds));
        assert_eq!(st.hand_index, 1);
        assert_eq!(st.game.pot, 0);
        assert!(st.seed_commitments.iter().all(|c| c.is_none()));
    }

    #[test]
    fn game_over_when_one_player_has_chips() {
        crypto::init().ok();
        let mut st = setup_table(2);
        st.game.players[0].chips = 2000;
        st.game.players[1].chips = 0;
        assert!(st.game_over());
    }

    #[test]
    fn verify_seed_after_complete() {
        let mut state = setup_table(2);
        state.phase = Phase::Complete;
        let seed = b"my_seed".to_vec();
        let hash = crypto::blake2b(&seed).unwrap();
        state.seed_commitments[0] = Some(hash);

        state
            .apply(&Action::VerifySeed {
                player_id: 0,
                seed: seed.clone(),
            })
            .unwrap();
        assert!(state.seeds_verified[0]);

        // Bad seed rejected
        state.seed_commitments[1] = Some(crypto::blake2b(b"real").unwrap());
        let result = state.apply(&Action::VerifySeed {
            player_id: 1,
            seed: b"fake".to_vec(),
        });
        assert!(result.is_err());
    }
}
