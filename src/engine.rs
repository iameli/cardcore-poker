//! Game-agnostic mental-card crypto engine.
//!
//! `CryptoRound` tracks the cryptographic state of one dealt round of any card
//! game built on the commit → shuffle → lock → reveal protocol:
//!
//! 1. Commit seeds (hash only — seeds stay secret)
//! 2. Shuffle: each live player encrypts all cards with one key, permutes
//! 3. Lock: each live player swaps the shuffle key for per-position lock keys
//! 4. Deal: players reveal per-position lock scalars (optionally excluding the
//!    card's recipient, so only they can decrypt their own card)
//! 5. After the round: reveal seeds for full game replay and verification
//!
//! Game rules (wagering, turn order, settlement) live in the per-game state
//! machines; this module only owns the deck and the crypto bookkeeping.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::crypto::{self, Point, Scalar};

/// Seat index within the table roster.
pub type PlayerId = usize;

/// Every round plays with a single full deck.
pub const DECK_SIZE: usize = 52;

/// Crypto bookkeeping for one round (hand) of cards.
///
/// `live` parameters are the seats still in the game, in seat order — only
/// those players participate in the protocol (commit/shuffle/lock/reveal).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoRound {
    /// The encrypted deck after shuffling/locking.
    pub deck: Vec<Point>,
    /// Each seat's committed (hashed) seed for this round.
    pub seed_commitments: Vec<Option<[u8; crypto::HASH_BYTES]>>,
    pub shuffles_done: usize,
    pub locks_done: usize,
    /// Scalars revealed so far for the card currently being dealt.
    pub deal_reveals: HashMap<PlayerId, Scalar>,
    /// Sequential cursor: the next undealt deck position.
    pub next_deck_position: usize,
    /// Post-round seed verification tracking.
    pub seeds_verified: Vec<bool>,
}

impl CryptoRound {
    pub fn new(num_players: usize) -> Self {
        Self {
            deck: Vec::new(),
            seed_commitments: vec![None; num_players],
            shuffles_done: 0,
            locks_done: 0,
            deal_reveals: HashMap::new(),
            next_deck_position: 0,
            seeds_verified: vec![false; num_players],
        }
    }

    /// Clear per-round state so the next round can start at seed commitment.
    /// Verification flags intentionally survive across rounds (a seed, once
    /// revealed and verified, stays verified).
    pub fn reset_for_next_hand(&mut self) {
        let n = self.seed_commitments.len();
        self.seed_commitments = vec![None; n];
        self.shuffles_done = 0;
        self.locks_done = 0;
        self.deal_reveals.clear();
        self.next_deck_position = 0;
        self.deck.clear();
    }

    /// Record a seed commitment. Returns `true` once every live seat has
    /// committed — the caller then seeds `deck` (e.g. from
    /// `crypto::card_points()`) and starts the shuffle.
    pub fn apply_commit_seed(
        &mut self,
        player_id: PlayerId,
        commitment: [u8; crypto::HASH_BYTES],
        live: &[usize],
    ) -> crate::Result<bool> {
        if !live.contains(&player_id) {
            return Err(crate::Error::InvalidAction(
                "eliminated players don't participate in the protocol".into(),
            ));
        }
        if self.seed_commitments[player_id].is_some() {
            return Err(crate::Error::InvalidAction("already committed".into()));
        }
        self.seed_commitments[player_id] = Some(commitment);
        let all_live_committed = self
            .seed_commitments
            .iter()
            .enumerate()
            .all(|(i, c)| !live.contains(&i) || c.is_some());
        Ok(all_live_committed)
    }

    /// Accept a player's shuffled deck. Returns the next seat to shuffle, or
    /// `None` once every live seat has shuffled (locking starts at `live[0]`).
    pub fn apply_shuffle(
        &mut self,
        player_id: PlayerId,
        expected: PlayerId,
        deck: &[Point],
        live: &[usize],
    ) -> crate::Result<Option<PlayerId>> {
        if player_id != expected {
            return Err(crate::Error::InvalidAction(
                "not your turn to shuffle".into(),
            ));
        }
        if deck.len() != DECK_SIZE {
            return Err(crate::Error::InvalidAction(
                "deck must have 52 cards".into(),
            ));
        }
        self.deck = deck.to_vec();
        self.shuffles_done += 1;
        if self.shuffles_done >= live.len() {
            Ok(None)
        } else {
            Ok(Some(live[self.shuffles_done]))
        }
    }

    /// Accept a player's locked deck. Returns the next seat to lock, or `None`
    /// once every live seat has locked (positions are now fixed — deal away).
    pub fn apply_lock(
        &mut self,
        player_id: PlayerId,
        expected: PlayerId,
        deck: &[Point],
        live: &[usize],
    ) -> crate::Result<Option<PlayerId>> {
        if player_id != expected {
            return Err(crate::Error::InvalidAction("not your turn to lock".into()));
        }
        if deck.len() != DECK_SIZE {
            return Err(crate::Error::InvalidAction(
                "deck must have 52 cards".into(),
            ));
        }
        self.deck = deck.to_vec();
        self.locks_done += 1;
        if self.locks_done >= live.len() {
            Ok(None)
        } else {
            Ok(Some(live[self.locks_done]))
        }
    }

    /// Start dealing the card at the cursor: clear reveal accounting and
    /// return the deck position being dealt.
    pub fn begin_deal(&mut self) -> usize {
        self.deal_reveals.clear();
        self.next_deck_position
    }

    /// Record one player's lock-scalar reveal for the card at
    /// `expected_position`. `exclude` is the card's recipient for a private
    /// deal (they don't reveal — their lock key stays on the card), or `None`
    /// for a public deal. Once every required live seat has revealed, the
    /// accumulated scalars are applied, the cursor advances, and the resulting
    /// point is returned (still lock-encrypted by the recipient for a private
    /// deal; fully decrypted for a public one).
    pub fn apply_reveal(
        &mut self,
        player_id: PlayerId,
        deck_position: usize,
        expected_position: usize,
        scalar: &Scalar,
        exclude: Option<PlayerId>,
        live: &[usize],
    ) -> crate::Result<Option<Point>> {
        if deck_position != expected_position {
            return Err(crate::Error::InvalidAction("wrong deck position".into()));
        }
        if exclude == Some(player_id) {
            return Err(crate::Error::InvalidAction(
                "recipient doesn't reveal for their own card".into(),
            ));
        }
        if !live.contains(&player_id) {
            // An eliminated player never locked this deck — their scalar
            // would corrupt the decryption.
            return Err(crate::Error::InvalidAction(
                "eliminated players don't participate in the protocol".into(),
            ));
        }
        if self.deal_reveals.contains_key(&player_id) {
            return Err(crate::Error::InvalidAction("already revealed".into()));
        }

        self.deal_reveals.insert(player_id, scalar.clone());

        // Only live players locked the deck, so only their reveals are needed
        // (minus the recipient, for a private deal).
        let reveals_needed = match exclude {
            Some(_) => live.len() - 1,
            None => live.len(),
        };

        if self.deal_reveals.len() >= reveals_needed {
            let mut point = self.deck[expected_position].clone();
            for s in self.deal_reveals.values() {
                point = crypto::decrypt(&point, s)?;
            }
            self.next_deck_position += 1;
            self.deal_reveals.clear();
            Ok(Some(point))
        } else {
            Ok(None)
        }
    }

    /// Verify a revealed seed against its commitment. Actual replay
    /// verification happens client-side — with the seed, every key the player
    /// derived during the round can be reproduced and checked.
    pub fn apply_verify_seed(&mut self, player_id: PlayerId, seed: &[u8]) -> crate::Result<()> {
        if self.seeds_verified[player_id] {
            return Err(crate::Error::InvalidAction("already verified".into()));
        }
        let hash = crypto::blake2b(seed)?;
        let commitment = self.seed_commitments[player_id]
            .ok_or_else(|| crate::Error::InvalidAction("no commitment found".into()))?;
        if hash != commitment {
            return Err(crate::Error::Crypto("seed doesn't match commitment".into()));
        }
        self.seeds_verified[player_id] = true;
        Ok(())
    }
}
