//! Ristretto255-based commutative encryption for mental poker.
//!
//! Each card is mapped to a Ristretto255 curve point. "Encrypting" a card means
//! multiplying its point by a secret scalar. Because scalar multiplication is
//! commutative, multiple players can encrypt in any order and decrypt in any order:
//!
//!   k_a * (k_b * P) = k_b * (k_a * P)
//!
//! All operations use libsodium's Ristretto255 API — no custom crypto.

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
        // Show first 8 bytes as hex for debugging
        let hex: String = self.0[..8].iter().map(|b| format!("{:02x}", b)).collect();
        write!(f, "Point({})", hex)
    }
}

impl std::fmt::Debug for Scalar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Scalar(***)")
    }
}

/// A player's encryption key pair.
/// Single key per player — used to encrypt all cards during shuffle.
/// During dealing, players apply their decryption to specific cards
/// and send the resulting point (not the scalar) to avoid leaking the key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerKeys {
    pub encrypt: Scalar,
    pub decrypt: Scalar,
}

impl PlayerKeys {
    /// Generate a fresh random key pair.
    pub fn generate() -> crate::Result<Self> {
        init()?;
        let (encrypt, decrypt) = generate_keypair()?;
        Ok(Self { encrypt, decrypt })
    }

    /// Encrypt all cards in a deck with this player's key.
    pub fn encrypt_deck(&self, deck: &[Point]) -> crate::Result<Vec<Point>> {
        deck.iter().map(|p| encrypt(p, &self.encrypt)).collect()
    }

    /// Decrypt a single card point with this player's key.
    pub fn decrypt_point(&self, point: &Point) -> crate::Result<Point> {
        decrypt(point, &self.decrypt)
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
    // Hash the card's canonical string representation to 64 bytes,
    // then use ristretto255_from_hash to map to a curve point.
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
/// This is the same operation as encrypt, just with the inverse scalar.
pub fn decrypt(point: &Point, inverse_scalar: &Scalar) -> crate::Result<Point> {
    encrypt(point, inverse_scalar)
}

/// Hash arbitrary data with BLAKE2b (used for RNG seed commitment, state hashing, etc.)
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

        // Encrypt A then B
        let ab = encrypt(&encrypt(&point, &k_a).unwrap(), &k_b).unwrap();
        // Encrypt B then A
        let ba = encrypt(&encrypt(&point, &k_b).unwrap(), &k_a).unwrap();
        assert_eq!(ab, ba);

        // Decrypt in opposite order: remove A first, then B
        let dec_a_first = decrypt(&decrypt(&ab, &d_a).unwrap(), &d_b).unwrap();
        assert_eq!(dec_a_first, point);

        // Decrypt B first, then A
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
}
