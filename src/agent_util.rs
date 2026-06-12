//! Helpers shared by the per-game player agents.
//!
//! The agents (poker's `PlayerAgent`, blackjack's `BlackjackAgent`) drive
//! their game's protocol state machine from AT Protocol records. The
//! cryptographic responses they auto-generate are identical across games and
//! live here as pure functions.

use rand::prelude::SliceRandom;

use crate::crypto::{self, PlayerKeys, PlayerRng, Point, Scalar};
use crate::engine::{DECK_SIZE, PlayerId};

/// Derive a per-hand secret seed from the master seed and hand index. Using a
/// one-way hash means a revealed per-hand seed can't be used to recover the
/// master seed or any other hand's seed.
pub fn per_hand_seed(master_seed: &[u8], hand_index: u64) -> crate::Result<Vec<u8>> {
    let mut data = Vec::with_capacity(master_seed.len() + 8);
    data.extend_from_slice(master_seed);
    data.extend_from_slice(&hand_index.to_le_bytes());
    Ok(crypto::blake2b(&data)?.to_vec())
}

/// The commitment published for a per-hand seed.
pub fn seed_commitment(per_hand_seed: &[u8]) -> crate::Result<[u8; crypto::HASH_BYTES]> {
    crypto::blake2b(per_hand_seed)
}

/// Build the shuffle response: encrypt every card with the player's shuffle
/// key and permute the deck deterministically from the per-hand seed.
pub fn shuffle_deck_response(
    keys: &PlayerKeys,
    per_hand_seed: &[u8],
    deck: &[Point],
) -> crate::Result<Vec<Point>> {
    let mut encrypted = keys.encrypt_deck(deck)?;
    let mut rng = PlayerRng::new(per_hand_seed, b"shuffle_permutation")?;
    encrypted.shuffle(rng.as_rng());
    Ok(encrypted)
}

/// Build the lock response: derive per-position lock keys (bound to the
/// current deck through a hash context so they can't be precomputed), remove
/// the shuffle key, and re-encrypt each position with its lock key. Mutates
/// `keys` so the lock scalars are available for later reveals.
pub fn lock_deck_response(
    keys: &mut PlayerKeys,
    per_hand_seed: &[u8],
    deck: &[Point],
) -> crate::Result<Vec<Point>> {
    let deck_hash = crypto::blake2b(&serde_json::to_vec(deck).unwrap())?;
    let mut context = b"lock:".to_vec();
    context.extend_from_slice(&deck_hash);
    let mut rng = PlayerRng::new(per_hand_seed, &context)?;
    keys.generate_lock_keys(DECK_SIZE, &mut rng)?;
    keys.lock_deck(deck)
}

/// The reveal for a deck position: this player's lock decryption scalar.
pub fn reveal_scalar(keys: &PlayerKeys, deck_position: usize) -> Scalar {
    keys.lock_decrypt[deck_position].clone()
}

/// Find which player should be performing the action matching `predicate`,
/// given the state machine's current valid actions.
pub fn find_player_for_action<A>(
    valid: &[A],
    predicate: impl Fn(&A) -> bool,
    player_of: impl Fn(&A) -> PlayerId,
) -> crate::Result<PlayerId> {
    valid
        .iter()
        .find(|a| predicate(a))
        .map(player_of)
        .ok_or_else(|| crate::Error::InvalidAction("no valid action of this type".into()))
}
