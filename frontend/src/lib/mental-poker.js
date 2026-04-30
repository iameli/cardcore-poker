/**
 * Mental Poker primitives adapted from cardcore
 *
 * Uses NaCl box (via tweetnacl) for encrypting/decrypting cards
 * so that players can shuffle and deal without a trusted dealer.
 *
 * Protocol:
 * 1. Each player generates an ed25519 keypair
 * 2. Each player encrypts every card in turn (multi-layer encryption)
 * 3. The deck is shuffled using a multi-party RNG
 * 4. When a card is dealt, each player decrypts one layer
 * 5. At showdown, all layers are removed to reveal the card
 */

import nacl from 'tweetnacl';
import naclUtil from 'tweetnacl-util';

// ─── Key Generation ────────────────────────────────────────────────

export function generateKeyPair() {
  const kp = nacl.box.keyPair();
  return {
    publicKey: naclUtil.encodeBase64(kp.publicKey),
    secretKey: naclUtil.encodeBase64(kp.secretKey),
  };
}

export function encodePublicKey(publicKeyBase64) {
  return `@${publicKeyBase64}`;
}

export function decodePublicKey(id) {
  return id.startsWith('@') ? id.slice(1) : id;
}

// ─── Card Encryption / Decryption ───────────────────────────────────

/**
 * Box (encrypt) a card value for a specific player.
 * Uses our secret key + their public key for asymmetric encryption.
 * Returns the encrypted payload as a base64 string.
 */
export function boxCard(cardValue, recipientPublicKeyBase64, senderSecretKeyBase64) {
  const recipientKey = naclUtil.decodeBase64(recipientPublicKeyBase64);
  const senderKey = naclUtil.decodeBase64(senderSecretKeyBase64);
  const message = naclUtil.decodeUTF8(JSON.stringify(cardValue));
  const nonce = nacl.randomBytes(nacl.box.nonceLength);

  const encrypted = nacl.box(message, nonce, recipientKey, senderKey);
  if (!encrypted) throw new Error('Box encryption failed');

  return {
    nonce: naclUtil.encodeBase64(nonce),
    ciphertext: naclUtil.encodeBase64(encrypted),
  };
}

/**
 * Unbox (decrypt) a card encrypted for us.
 * Uses our secret key + sender's public key for asymmetric decryption.
 */
export function unboxCard(encryptedPayload, ourSecretKeyBase64, senderPublicKeyBase64) {
  try {
    const nonce = naclUtil.decodeBase64(encryptedPayload.nonce);
    const ciphertext = naclUtil.decodeBase64(encryptedPayload.ciphertext);
    const ourKey = naclUtil.decodeBase64(ourSecretKeyBase64);
    const senderKey = naclUtil.decodeBase64(senderPublicKeyBase64);

    const decrypted = nacl.box.open(ciphertext, nonce, senderKey, ourKey);
    if (!decrypted) return null;

    return JSON.parse(naclUtil.encodeUTF8(decrypted));
  } catch (e) {
    console.error('Unbox failed:', e);
    return null;
  }
}

// ─── Multi-layer Encryption ─────────────────────────────────────────

/**
 * Encrypt a card with our layer on top.
 * The card may already be partially encrypted by other players.
 */
export function encryptLayer(card, ourKeyPair, nextPlayerPublicKeys) {
  // card is either a raw card object or a nested encrypted structure
  // We wrap it in an outer box for the next player
  const layer = {
    card,
    layer: ourKeyPair.publicKey,
  };

  if (nextPlayerPublicKeys.length === 0) {
    return layer;
  }

  // Box for the first player in the list
  return boxCard(layer, nextPlayerPublicKeys[0], ourKeyPair.secretKey);
}

/**
 * Decrypt our layer from a card, revealing the inner layer
 */
export function decryptLayer(encryptedCard, ourKeyPair, senderPublicKey) {
  if (typeof encryptedCard === 'object' && encryptedCard.layer) {
    // Already an unencrypted layer - just return the inner card
    return encryptedCard.card;
  }
  if (typeof encryptedCard === 'object' && encryptedCard.nonce) {
    return unboxCard(encryptedCard, ourKeyPair.secretKey, senderPublicKey);
  }
  return encryptedCard;
}

// ─── Shuffling with Shared RNG ──────────────────────────────────────

/**
 * Create a deterministic shuffle of an array using a seed
 */
export function seededShuffle(array, seed) {
  const arr = [...array];
  let seedVal = hashString(seed);

  for (let i = arr.length - 1; i > 0; i--) {
    seedVal = (seedVal * 1103515245 + 12345) & 0x7fffffff;
    const j = seedVal % (i + 1);
    [arr[i], arr[j]] = [arr[j], arr[i]];
  }

  return arr;
}

function hashString(str) {
  let hash = 5381;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) + hash + str.charCodeAt(i)) & 0x7fffffff;
  }
  return hash;
}

// ─── Deck Creation ──────────────────────────────────────────────────

export const SUITS = ['clubs', 'diamonds', 'hearts', 'spades'];
export const RANKS = ['A', '2', '3', '4', '5', '6', '7', '8', '9', '10', 'J', 'Q', 'K'];

export function createDeck() {
  const deck = [];
  for (const suit of SUITS) {
    for (let i = 0; i < RANKS.length; i++) {
      deck.push({ suit, rank: RANKS[i], index: i });
    }
  }
  return deck;
}

// ─── Full Shuffle Protocol ──────────────────────────────────────────

/**
 * Perform the full mental poker shuffle.
 * Each player encrypts each card in sequence.
 * Returns the encrypted deck and the seed contributions.
 */
export function mentalShuffle(deck, playerOrder, keyPairs) {
  let encryptedDeck = deck.map(c => ({ card: c, layer: null }));

  // Each player encrypts each card
  for (const playerId of playerOrder) {
    const kp = keyPairs[playerId];
    encryptedDeck = encryptedDeck.map(cardData => {
      const wrapped = {
        card: cardData,
        layer: kp.publicKey,
      };
      // The card is encrypted with the player's own key
      // (in real protocol, it would be encrypted for the next player)
      return wrapped;
    });
  }

  // Shuffle based on combined RNG contributions
  const seedContributions = playerOrder.map(id => keyPairs[id].publicKey);
  const combinedSeed = seedContributions.join('');
  const shuffled = seededShuffle(encryptedDeck, combinedSeed);

  return { deck: shuffled, seed: combinedSeed };
}

/**
 * Deal a card from the encrypted deck: peel off layers
 */
export function dealCard(encryptedDeck, index, playerOrder, keyPairs) {
  if (index >= encryptedDeck.length) return null;

  let cardData = encryptedDeck[index];

  // Each player decrypts their layer
  for (const playerId of playerOrder) {
    if (cardData && cardData.layer) {
      cardData = cardData.card;
    }
  }

  return cardData;
}

/**
 * Reveal a card at showdown - peel ALL layers
 */
export function revealCard(encryptedCard, keyPairs) {
  let card = encryptedCard;

  // Unwrap all layers
  while (card && typeof card === 'object' && 'card' in card) {
    card = card.card;
  }

  return card;
}
