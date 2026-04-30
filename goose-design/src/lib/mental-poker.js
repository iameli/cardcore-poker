/**
 * Mental Poker — re-exports from cardcore-wasm.js
 *
 * This file exists for backward compatibility. All crypto has moved
 * to the Rust WASM module (Ristretto255) via cardcore-wasm.js.
 *
 * Previously used NaCl (tweetnacl) — now fully replaced.
 */

export {
  SUITS,
  RANKS,
  createDeck,
  parseCard,
  generateSeed,
  hashSeed,
  initWasm,
} from './cardcore-wasm.js';

// ─── Key Pair (compatibility shim) ─────────────────────────────────

/**
 * @deprecated Use createAgent() from cardcore-wasm.js instead.
 * Returns a placeholder; real crypto happens in WASM.
 */
export function generateKeyPair() {
  console.warn('[mental-poker] generateKeyPair is deprecated. Key management is now in WASM.');
  // Return a placeholder — GameRoom uses this just for local identification.
  // The actual encryption keys are managed by WasmAgent internally.
  const arr = new Uint8Array(32);
  crypto.getRandomValues(arr);
  const b64 = btoa(String.fromCharCode(...arr));
  return { publicKey: b64, secretKey: b64 };
}

/**
 * @deprecated No longer needed; DIDs identify players in AT Protocol.
 */
export function encodePublicKey(key) {
  return `@${key}`;
}

/**
 * @deprecated No longer needed; DIDs identify players in AT Protocol.
 */
export function decodePublicKey(id) {
  return id.startsWith('@') ? id.slice(1) : id;
}

// ─── Card Encryption (compatibility shims) ─────────────────────────

/**
 * @deprecated Use CardcoreSession from cardcore-wasm.js instead.
 * Encrypt a card object as JSON → base64 (no-op shim).
 */
export function boxCard(cardValue, _recipientKey, _senderKey) {
  const json = JSON.stringify(cardValue);
  // Random 24-byte nonce per NaCl box convention
  const nonceBytes = new Uint8Array(24);
  crypto.getRandomValues(nonceBytes);
  const nonce = btoa(String.fromCharCode(...nonceBytes));
  return { nonce, ciphertext: btoa(json) };
}

/**
 * @deprecated Use CardcoreSession from cardcore-wasm.js instead.
 * Decrypt a base64 → JSON card object (no-op shim).
 */
export function unboxCard(encryptedPayload, _ourKey, _senderKey) {
  try {
    if (!encryptedPayload || !encryptedPayload.ciphertext) return null;
    const json = atob(encryptedPayload.ciphertext);
    return JSON.parse(json);
  } catch {
    return null;
  }
}

// ─── Layer Encryption (compatibility shims) ────────────────────────

/** @deprecated */
export function encryptLayer(card, ourKeyPair, _nextKeys) {
  return { card, layer: ourKeyPair.publicKey };
}

/** @deprecated */
export function decryptLayer(encryptedCard, _ourKeyPair, _senderKey) {
  if (encryptedCard && encryptedCard.card) return encryptedCard.card;
  return encryptedCard;
}

// ─── Shuffle (compatibility shim) ──────────────────────────────────

/** @deprecated Use CardcoreSession from cardcore-wasm.js */
export function seededShuffle(array, _seed) {
  // Fisher-Yates with Math.random (non-cryptographic, for UI only)
  const arr = [...array];
  for (let i = arr.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [arr[i], arr[j]] = [arr[j], arr[i]];
  }
  return arr;
}

/** @deprecated */
export function mentalShuffle(deck, playerOrder, keyPairs) {
  let encryptedDeck = deck.map(c => ({ card: c, layer: null }));
  for (const playerId of playerOrder) {
    const kp = keyPairs[playerId];
    encryptedDeck = encryptedDeck.map(cardData => ({
      card: cardData,
      layer: kp?.publicKey || '',
    }));
  }
  return { deck: encryptedDeck, seed: '' };
}

/** @deprecated */
export function dealCard(encryptedDeck, index, _playerOrder, _keyPairs) {
  if (index >= encryptedDeck.length) return null;
  let cardData = encryptedDeck[index];
  while (cardData && cardData.card) cardData = cardData.card;
  return cardData;
}

/** @deprecated */
export function revealCard(encryptedCard, _keyPairs) {
  let card = encryptedCard;
  while (card && card.card) card = card.card;
  return card;
}
