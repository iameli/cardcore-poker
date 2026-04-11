//! Ristretto255-based commutative encryption for mental poker.
//!
//! Each card is mapped to a Ristretto255 curve point. "Encrypting" a card means
//! multiplying its point by a secret scalar. Because scalar multiplication is
//! commutative, multiple players can encrypt in any order and decrypt in any order:
//!
//!   k_a * (k_b * P) = k_b * (k_a * P)
//!
//! Two-phase protocol:
//! 1. Shuffle phase: each player encrypts all cards with ONE scalar, shuffles.
//! 2. Lock phase: each player removes their shuffle key and re-encrypts each card
//!    with a unique per-position lock key.
//!
//! Dealing reveals per-position lock scalars — verifiable by anyone.

use libsodium_sys::*;
use serde::{Deserialize, Serialize};
use std::sync::Once;

use crate::card::Card;

static SODIUM_INIT: Once = Once::new();

/// Initialize libsodium. Safe to call multiple times.
pub fn init() -> crate::Result<()> {
    let mut result = Ok(());
    SODIUM_INIT.call_once(|| unsafe {
        if sodium_init() < 0 {
            result = Err(crate::Error::SodiumInit);
        }
    });
    result
}

pub const SCALAR_BYTES: usize = crypto_core_ristretto255_SCALARBYTES as usize;
pub const POINT_BYTES: usize = crypto_core_ristretto255_BYTES as usize;
pub const HASH_BYTES: usize = crypto_generichash_BYTES as usize;

/// A secret scalar used to encrypt/decrypt cards.
#[derive(Clone, Serialize, Deserialize)]
pub struct Scalar(pub [u8; SCALAR_BYTES]);

/// A Ristretto255 point representing an encrypted (or plaintext) card.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Point(pub [u8; POINT_BYTES]);

impl std::fmt::Debug for Point {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex: String = self.0[..8].iter().map(|b| format!("{:02x}", b)).collect();
        write!(f, "Point({})", hex)
    }
}

impl std::fmt::Debug for Scalar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Scalar(***)")
    }
}

/// A player's keys for the two-phase protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerKeys {
    /// Single key used to encrypt all cards during shuffle.
    pub shuffle_encrypt: Scalar,
    pub shuffle_decrypt: Scalar,
    /// Per-position lock keys (generated during lock phase, one per card).
    pub lock_encrypt: Vec<Scalar>,
    pub lock_decrypt: Vec<Scalar>,
}

impl PlayerKeys {
    /// Generate shuffle keys. Lock keys are generated later during the lock phase.
    pub fn generate() -> crate::Result<Self> {
        init()?;
        let (shuffle_encrypt, shuffle_decrypt) = generate_keypair()?;
        Ok(Self {
            shuffle_encrypt,
            shuffle_decrypt,
            lock_encrypt: Vec::new(),
            lock_decrypt: Vec::new(),
        })
    }

    /// Generate per-position lock keys for `n` cards.
    pub fn generate_lock_keys(&mut self, n: usize) -> crate::Result<()> {
        init()?;
        self.lock_encrypt = Vec::with_capacity(n);
        self.lock_decrypt = Vec::with_capacity(n);
        for _ in 0..n {
            let (e, d) = generate_keypair()?;
            self.lock_encrypt.push(e);
            self.lock_decrypt.push(d);
        }
        Ok(())
    }

    /// Shuffle phase: encrypt all cards with the single shuffle key.
    pub fn encrypt_deck(&self, deck: &[Point]) -> crate::Result<Vec<Point>> {
        deck.iter()
            .map(|p| encrypt(p, &self.shuffle_encrypt))
            .collect()
    }

    /// Lock phase: remove shuffle encryption and apply per-position lock key.
    /// Input deck must still have this player's shuffle encryption on it.
    pub fn lock_deck(&self, deck: &[Point]) -> crate::Result<Vec<Point>> {
        deck.iter()
            .enumerate()
            .map(|(i, p)| {
                // Remove shuffle key
                let unlocked = decrypt(p, &self.shuffle_decrypt)?;
                // Apply position lock key
                encrypt(&unlocked, &self.lock_encrypt[i])
            })
            .collect()
    }
}

/// Generate a random scalar and its multiplicative inverse.
pub fn generate_keypair() -> crate::Result<(Scalar, Scalar)> {
    init()?;
    let mut enc = Scalar([0u8; SCALAR_BYTES]);
    let mut dec = Scalar([0u8; SCALAR_BYTES]);
    unsafe {
        crypto_core_ristretto255_scalar_random(enc.0.as_mut_ptr());
        if crypto_core_ristretto255_scalar_invert(dec.0.as_mut_ptr(), enc.0.as_ptr()) != 0 {
            return Err(crate::Error::Crypto("scalar invert failed".into()));
        }
    }
    Ok((enc, dec))
}

/// Map a card to a unique Ristretto255 point via hash-to-group.
pub fn card_to_point(card: &Card) -> crate::Result<Point> {
    init()?;
    let label = format!("cardcore-poker:card:{}", card);
    let mut hash = [0u8; 64];
    unsafe {
        crypto_generichash(
            hash.as_mut_ptr(),
            64,
            label.as_ptr(),
            label.len() as u64,
            std::ptr::null(),
            0,
        );
    }
    let mut point = Point([0u8; POINT_BYTES]);
    unsafe {
        crypto_core_ristretto255_from_hash(point.0.as_mut_ptr(), hash.as_ptr());
    }
    Ok(point)
}

/// Build the mapping of all 52 cards to their Ristretto255 points.
pub fn card_points() -> crate::Result<Vec<(Card, Point)>> {
    Card::deck()
        .into_iter()
        .map(|c| {
            let p = card_to_point(&c)?;
            Ok((c, p))
        })
        .collect()
}

/// Encrypt (lock) a point by multiplying by a scalar.
pub fn encrypt(point: &Point, scalar: &Scalar) -> crate::Result<Point> {
    init()?;
    let mut out = Point([0u8; POINT_BYTES]);
    unsafe {
        if crypto_scalarmult_ristretto255(out.0.as_mut_ptr(), scalar.0.as_ptr(), point.0.as_ptr())
            != 0
        {
            return Err(crate::Error::Crypto("scalarmult failed".into()));
        }
    }
    Ok(out)
}

/// Decrypt (unlock) a point by multiplying by the scalar's inverse.
pub fn decrypt(point: &Point, inverse_scalar: &Scalar) -> crate::Result<Point> {
    encrypt(point, inverse_scalar)
}

/// Hash arbitrary data with BLAKE2b.
pub fn blake2b(data: &[u8]) -> crate::Result<[u8; HASH_BYTES]> {
    init()?;
    let mut out = [0u8; HASH_BYTES];
    unsafe {
        crypto_generichash(
            out.as_mut_ptr(),
            HASH_BYTES,
            data.as_ptr(),
            data.len() as u64,
            std::ptr::null(),
            0,
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let card = Card::new(Rank::Ace, Suit::Spades);
        let point = card_to_point(&card).unwrap();
        let (enc_key, dec_key) = generate_keypair().unwrap();

        let encrypted = encrypt(&point, &enc_key).unwrap();
        assert_ne!(encrypted, point);

        let decrypted = decrypt(&encrypted, &dec_key).unwrap();
        assert_eq!(decrypted, point);
    }

    #[test]
    fn commutativity() {
        let card = Card::new(Rank::King, Suit::Hearts);
        let point = card_to_point(&card).unwrap();

        let (k_a, d_a) = generate_keypair().unwrap();
        let (k_b, d_b) = generate_keypair().unwrap();

        let ab = encrypt(&encrypt(&point, &k_a).unwrap(), &k_b).unwrap();
        let ba = encrypt(&encrypt(&point, &k_b).unwrap(), &k_a).unwrap();
        assert_eq!(ab, ba);

        let dec_a_first = decrypt(&decrypt(&ab, &d_a).unwrap(), &d_b).unwrap();
        assert_eq!(dec_a_first, point);

        let dec_b_first = decrypt(&decrypt(&ab, &d_b).unwrap(), &d_a).unwrap();
        assert_eq!(dec_b_first, point);
    }

    #[test]
    fn all_cards_unique_points() {
        let points = card_points().unwrap();
        assert_eq!(points.len(), 52);
        let unique: std::collections::HashSet<_> = points.iter().map(|(_, p)| p.clone()).collect();
        assert_eq!(unique.len(), 52);
    }

    #[test]
    fn two_phase_roundtrip() {
        // Simulate the full two-phase protocol with 2 players
        let card = Card::new(Rank::Ace, Suit::Spades);
        let point = card_to_point(&card).unwrap();

        let mut alice = PlayerKeys::generate().unwrap();
        let mut bob = PlayerKeys::generate().unwrap();

        // Shuffle phase: both encrypt with shuffle keys
        let after_alice = encrypt(&point, &alice.shuffle_encrypt).unwrap();
        let after_both = encrypt(&after_alice, &bob.shuffle_encrypt).unwrap();

        // Lock phase: each removes shuffle key, adds lock key
        alice.generate_lock_keys(1).unwrap();
        bob.generate_lock_keys(1).unwrap();

        // Alice locks (removes her shuffle, adds her lock)
        let alice_unlocked = decrypt(&after_both, &alice.shuffle_decrypt).unwrap();
        let alice_locked = encrypt(&alice_unlocked, &alice.lock_encrypt[0]).unwrap();

        // Bob locks (removes his shuffle, adds his lock)
        let bob_unlocked = decrypt(&alice_locked, &bob.shuffle_decrypt).unwrap();
        let bob_locked = encrypt(&bob_unlocked, &bob.lock_encrypt[0]).unwrap();

        // Now card is: lock_a_0 · lock_b_0 · P
        // To reveal: apply both lock decrypts (order doesn't matter)
        let remove_alice = decrypt(&bob_locked, &alice.lock_decrypt[0]).unwrap();
        let final_point = decrypt(&remove_alice, &bob.lock_decrypt[0]).unwrap();
        assert_eq!(final_point, point);

        // Other order works too
        let remove_bob = decrypt(&bob_locked, &bob.lock_decrypt[0]).unwrap();
        let final_point2 = decrypt(&remove_bob, &alice.lock_decrypt[0]).unwrap();
        assert_eq!(final_point2, point);
    }
}
