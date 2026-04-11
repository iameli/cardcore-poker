//! Ristretto255-based commutative encryption for mental poker.
//!
//! Pure Rust implementation using curve25519-dalek — compiles to WASM.
//!
//! Two-phase protocol:
//! 1. Shuffle phase: each player encrypts all cards with ONE scalar, shuffles.
//! 2. Lock phase: each player removes their shuffle key and re-encrypts each card
//!    with a unique per-position lock key.
//!
//! All randomness is deterministic from each player's secret seed, mixed with
//! public game context.

use blake2::digest::consts::U32;
use blake2::{Blake2b, Digest};
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar as DalekScalar;
use rand::RngCore;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};

use crate::card::Card;

type Blake2b256 = Blake2b<U32>;

pub const SCALAR_BYTES: usize = 32;
pub const POINT_BYTES: usize = 32;
pub const HASH_BYTES: usize = 32;

/// A secret scalar used to encrypt/decrypt cards.
#[derive(Clone)]
#[derive(Serialize, Deserialize)]
pub struct Scalar(#[serde(with = "serde_base64")] pub [u8; SCALAR_BYTES]);

/// A Ristretto255 point representing an encrypted (or plaintext) card.
#[derive(Clone, PartialEq, Eq, Hash)]
#[derive(Serialize, Deserialize)]
pub struct Point(#[serde(with = "serde_base64")] pub [u8; POINT_BYTES]);

pub mod serde_base64 {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        let s: &str = serde::Deserialize::deserialize(deserializer)?;
        let vec = STANDARD
            .decode(s)
            .map_err(serde::de::Error::custom)?;
        vec.try_into()
            .map_err(|_| serde::de::Error::custom(format!("expected {} bytes", N)))
    }
}

pub mod serde_base64_vec {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s: &str = serde::Deserialize::deserialize(deserializer)?;
        STANDARD.decode(s).map_err(serde::de::Error::custom)
    }
}

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

// --- Internal conversions ---

fn to_dalek_scalar(s: &Scalar) -> DalekScalar {
    let opt = DalekScalar::from_canonical_bytes(s.0);
    if bool::from(opt.is_some()) {
        opt.unwrap()
    } else {
        // Reduce mod group order if not canonical
        let mut wide = [0u8; 64];
        wide[..32].copy_from_slice(&s.0);
        DalekScalar::from_bytes_mod_order_wide(&wide)
    }
}

fn from_dalek_scalar(s: &DalekScalar) -> Scalar {
    Scalar(s.to_bytes())
}

fn to_dalek_point(p: &Point) -> Option<RistrettoPoint> {
    CompressedRistretto(p.0).decompress()
}

fn from_dalek_point(p: &RistrettoPoint) -> Point {
    Point(p.compress().to_bytes())
}

// --- PlayerRng ---

/// Deterministic RNG derived from a player's secret seed and game context.
pub struct PlayerRng {
    inner: ChaCha20Rng,
}

impl PlayerRng {
    /// Create a new RNG from a seed and domain context.
    pub fn new(seed: &[u8], context: &[u8]) -> crate::Result<Self> {
        let mut hasher = Blake2b256::new();
        hasher.update(seed);
        hasher.update(context);
        let hash: [u8; 32] = hasher.finalize().into();
        Ok(Self {
            inner: ChaCha20Rng::from_seed(hash),
        })
    }

    /// Generate a random Ristretto255 scalar deterministically.
    pub fn random_scalar(&mut self) -> crate::Result<Scalar> {
        let mut wide = [0u8; 64];
        self.inner.fill_bytes(&mut wide);
        let dalek = DalekScalar::from_bytes_mod_order_wide(&wide);
        Ok(from_dalek_scalar(&dalek))
    }

    /// Generate a scalar and its multiplicative inverse.
    pub fn random_keypair(&mut self) -> crate::Result<(Scalar, Scalar)> {
        let enc = self.random_scalar()?;
        let dalek_enc = to_dalek_scalar(&enc);
        let dalek_dec = dalek_enc.invert();
        Ok((enc, from_dalek_scalar(&dalek_dec)))
    }

    /// Get access to the inner RNG for shuffle permutations etc.
    pub fn as_rng(&mut self) -> &mut ChaCha20Rng {
        &mut self.inner
    }
}

// --- PlayerKeys ---

/// A player's keys for the two-phase protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerKeys {
    pub shuffle_encrypt: Scalar,
    pub shuffle_decrypt: Scalar,
    pub lock_encrypt: Vec<Scalar>,
    pub lock_decrypt: Vec<Scalar>,
}

impl PlayerKeys {
    pub fn generate(rng: &mut PlayerRng) -> crate::Result<Self> {
        let (shuffle_encrypt, shuffle_decrypt) = rng.random_keypair()?;
        Ok(Self {
            shuffle_encrypt,
            shuffle_decrypt,
            lock_encrypt: Vec::new(),
            lock_decrypt: Vec::new(),
        })
    }

    pub fn generate_lock_keys(&mut self, n: usize, rng: &mut PlayerRng) -> crate::Result<()> {
        self.lock_encrypt = Vec::with_capacity(n);
        self.lock_decrypt = Vec::with_capacity(n);
        for _ in 0..n {
            let (e, d) = rng.random_keypair()?;
            self.lock_encrypt.push(e);
            self.lock_decrypt.push(d);
        }
        Ok(())
    }

    pub fn encrypt_deck(&self, deck: &[Point]) -> crate::Result<Vec<Point>> {
        deck.iter()
            .map(|p| encrypt(p, &self.shuffle_encrypt))
            .collect()
    }

    pub fn lock_deck(&self, deck: &[Point]) -> crate::Result<Vec<Point>> {
        deck.iter()
            .enumerate()
            .map(|(i, p)| {
                let unlocked = decrypt(p, &self.shuffle_decrypt)?;
                encrypt(&unlocked, &self.lock_encrypt[i])
            })
            .collect()
    }
}

// --- Core operations ---

/// Generate a random scalar and its multiplicative inverse (non-deterministic).
pub fn generate_keypair() -> crate::Result<(Scalar, Scalar)> {
    let mut rng = rand::rng();
    let mut wide = [0u8; 64];
    rng.fill_bytes(&mut wide);
    let dalek_enc = DalekScalar::from_bytes_mod_order_wide(&wide);
    let dalek_dec = dalek_enc.invert();
    Ok((from_dalek_scalar(&dalek_enc), from_dalek_scalar(&dalek_dec)))
}

/// Map a card to a unique Ristretto255 point via hash-to-group.
pub fn card_to_point(card: &Card) -> crate::Result<Point> {
    let label = format!("cardcore-poker:card:{}", card);
    let point = RistrettoPoint::hash_from_bytes::<blake2::Blake2b512>(label.as_bytes());
    Ok(from_dalek_point(&point))
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
    let p = to_dalek_point(point)
        .ok_or_else(|| crate::Error::Crypto("invalid point".into()))?;
    let s = to_dalek_scalar(scalar);
    Ok(from_dalek_point(&(s * p)))
}

/// Decrypt (unlock) a point by multiplying by the scalar's inverse.
pub fn decrypt(point: &Point, inverse_scalar: &Scalar) -> crate::Result<Point> {
    encrypt(point, inverse_scalar)
}

/// Hash arbitrary data with BLAKE2b-256.
pub fn blake2b(data: &[u8]) -> crate::Result<[u8; HASH_BYTES]> {
    let mut hasher = Blake2b256::new();
    hasher.update(data);
    Ok(hasher.finalize().into())
}

/// No-op init (libsodium is gone, pure Rust now).
pub fn init() -> crate::Result<()> {
    Ok(())
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
    fn deterministic_rng_reproducible() {
        let seed = b"test_seed_123";
        let context = b"shuffle";

        let mut rng1 = PlayerRng::new(seed, context).unwrap();
        let mut rng2 = PlayerRng::new(seed, context).unwrap();

        let s1 = rng1.random_scalar().unwrap();
        let s2 = rng2.random_scalar().unwrap();
        assert_eq!(s1.0, s2.0);

        let mut rng3 = PlayerRng::new(seed, b"lock").unwrap();
        let s3 = rng3.random_scalar().unwrap();
        assert_ne!(s1.0, s3.0);
    }

    #[test]
    fn deterministic_keypair_roundtrip() {
        let mut rng = PlayerRng::new(b"my_secret_seed", b"test").unwrap();
        let (enc, dec) = rng.random_keypair().unwrap();

        let card = Card::new(Rank::Ace, Suit::Spades);
        let point = card_to_point(&card).unwrap();

        let encrypted = encrypt(&point, &enc).unwrap();
        let decrypted = decrypt(&encrypted, &dec).unwrap();
        assert_eq!(decrypted, point);
    }

    #[test]
    fn two_phase_roundtrip() {
        let card = Card::new(Rank::Ace, Suit::Spades);
        let point = card_to_point(&card).unwrap();

        let mut rng_a = PlayerRng::new(b"alice_seed", b"shuffle").unwrap();
        let mut rng_b = PlayerRng::new(b"bob_seed", b"shuffle").unwrap();
        let mut alice = PlayerKeys::generate(&mut rng_a).unwrap();
        let mut bob = PlayerKeys::generate(&mut rng_b).unwrap();

        let after_alice = encrypt(&point, &alice.shuffle_encrypt).unwrap();
        let after_both = encrypt(&after_alice, &bob.shuffle_encrypt).unwrap();

        let mut lock_rng_a = PlayerRng::new(b"alice_seed", b"lock").unwrap();
        let mut lock_rng_b = PlayerRng::new(b"bob_seed", b"lock").unwrap();
        alice.generate_lock_keys(1, &mut lock_rng_a).unwrap();
        bob.generate_lock_keys(1, &mut lock_rng_b).unwrap();

        let alice_unlocked = decrypt(&after_both, &alice.shuffle_decrypt).unwrap();
        let alice_locked = encrypt(&alice_unlocked, &alice.lock_encrypt[0]).unwrap();

        let bob_unlocked = decrypt(&alice_locked, &bob.shuffle_decrypt).unwrap();
        let bob_locked = encrypt(&bob_unlocked, &bob.lock_encrypt[0]).unwrap();

        let remove_alice = decrypt(&bob_locked, &alice.lock_decrypt[0]).unwrap();
        let final_point = decrypt(&remove_alice, &bob.lock_decrypt[0]).unwrap();
        assert_eq!(final_point, point);
    }

    #[test]
    fn full_replay_from_seeds() {
        let seed_a = b"alice_secret";
        let seed_b = b"bob_secret";

        let mut rng_a1 = PlayerRng::new(seed_a, b"shuffle").unwrap();
        let mut rng_b1 = PlayerRng::new(seed_b, b"shuffle").unwrap();
        let keys_a1 = PlayerKeys::generate(&mut rng_a1).unwrap();
        let keys_b1 = PlayerKeys::generate(&mut rng_b1).unwrap();

        let mut rng_a2 = PlayerRng::new(seed_a, b"shuffle").unwrap();
        let mut rng_b2 = PlayerRng::new(seed_b, b"shuffle").unwrap();
        let keys_a2 = PlayerKeys::generate(&mut rng_a2).unwrap();
        let keys_b2 = PlayerKeys::generate(&mut rng_b2).unwrap();

        assert_eq!(keys_a1.shuffle_encrypt.0, keys_a2.shuffle_encrypt.0);
        assert_eq!(keys_a1.shuffle_decrypt.0, keys_a2.shuffle_decrypt.0);
        assert_eq!(keys_b1.shuffle_encrypt.0, keys_b2.shuffle_encrypt.0);
        assert_eq!(keys_b1.shuffle_decrypt.0, keys_b2.shuffle_decrypt.0);
    }
}
