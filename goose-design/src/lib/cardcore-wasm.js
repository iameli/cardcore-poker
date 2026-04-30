/**
 * Cardcore WASM Bridge
 *
 * Replaces the NaCl-based mental poker with the Rust WASM module
 * using Ristretto255 commutative encryption and the two-phase
 * shuffle+lock protocol described in the README.
 *
 * All crypto happens in WASM. DAG-CBOR encoding uses @ipld/dag-cbor.
 * AT Protocol publishing uses @atcute/client.
 */

import init, { WasmAgent, simulate_game } from '../../../pkg/cardcore_poker.js';
import * as dagCbor from '@ipld/dag-cbor';

// ─── WASM Init ─────────────────────────────────────────────────────

let _wasmReady = false;
let _wasmInitPromise = null;

export async function initWasm() {
  if (_wasmReady) return;
  if (_wasmInitPromise) return _wasmInitPromise;
  _wasmInitPromise = init().then(() => {
    _wasmReady = true;
    console.log('[cardcore-wasm] WASM module initialized');
  });
  return _wasmInitPromise;
}

// ─── Player Agent ──────────────────────────────────────────────────

/**
 * Create a player agent bound to a DID and secret seed.
 */
export function createAgent(did, seed) {
  if (typeof seed === 'string') {
    seed = new TextEncoder().encode(seed);
  }
  if (!(seed instanceof Uint8Array)) {
    throw new Error('seed must be Uint8Array or string');
  }
  return new WasmAgent(did, seed);
}

/**
 * Encode a record object to DAG-CBOR bytes using @ipld/dag-cbor.
 */
export function encodeRecord(record) {
  return dagCbor.encode(record);
}

/**
 * Decode DAG-CBOR bytes back to a JS object.
 */
export function decodeRecord(cbor) {
  return dagCbor.decode(cbor);
}

// ─── Mental Poker Protocol (WASM-backed) ───────────────────────────

/**
 * A game session backed by WasmAgent instances.
 * Manages the full two-phase shuffle+lock protocol.
 */
export class CardcoreSession {
  /**
   * @param {object} opts
   * @param {string[]} opts.playerDids - Player DIDs in seat order
   * @param {Uint8Array[]} opts.seeds - Each player's seed
   * @param {number} opts.startingChips
   * @param {number} opts.smallBlind
   */
  constructor({ playerDids, seeds, startingChips = 1000, smallBlind = 10 }) {
    this.agents = {};
    this.playerOrder = playerDids;
    this.startingChips = BigInt(startingChips);
    this.smallBlind = BigInt(smallBlind);
    this.outputs = [];

    for (let i = 0; i < playerDids.length; i++) {
      const did = playerDids[i];
      const seed = seeds[i] || new Uint8Array(32);
      this.agents[did] = createAgent(did, seed);
    }
  }

  /**
   * Start the table — feed table record to all agents.
   */
  startTable() {
    const tableRecord = {
      $type: 're.cardco.poker.table',
      players: this.playerOrder,
      startingChips: Number(this.startingChips),
      smallBlind: Number(this.smallBlind),
      createdAt: new Date().toISOString(),
    };
    const tableCbor = encodeRecord(tableRecord);

    const allActions = [];
    for (const did of this.playerOrder) {
      const agent = this.agents[did];
      const output = agent.receive_table(tableCbor);
      this._collectActions(output, allActions);
    }
    return allActions;
  }

  /**
   * Feed an action CBOR to all agents.
   */
  processAction(actionCbor) {
    const allActions = [];
    for (const did of this.playerOrder) {
      const agent = this.agents[did];
      const output = agent.receive_action(actionCbor);
      this._collectActions(output, allActions);
    }
    return allActions;
  }

  /**
   * Submit a bet for a specific player.
   */
  submitBet(did, betAction) {
    const agent = this.agents[did];
    if (!agent) throw new Error(`Unknown player: ${did}`);
    const output = agent.bet(betAction);
    const actions = [];
    this._collectActions(output, actions);
    return actions;
  }

  /** Get hole cards for a player as parsed array. */
  getHoleCards(did) {
    const agent = this.agents[did];
    if (!agent) return [];
    return JSON.parse(agent.hole_cards());
  }

  /** Get community cards as parsed array. */
  getCommunityCards(did) {
    const agent = this.agents[did];
    if (!agent) return [];
    return JSON.parse(agent.community_cards());
  }

  /** Check if a player needs to make a bet decision. */
  checkStatus(did) {
    const agent = this.agents[did];
    if (!agent) return { kind: 'waiting', options: [] };
    const output = agent.check_status();
    return {
      kind: output.kind,
      options: output.kind === 'need_bet' ? JSON.parse(output.bet_options) : [],
    };
  }

  /** Run a complete simulated game (for testing/replay). */
  simulate(numPlayers, chips, blind, strategy = 'random', seed = 42n) {
    return simulate_game(numPlayers, chips, blind, strategy, seed);
  }

  _collectActions(output, target) {
    if (output.kind === 'actions') {
      for (let i = 0; i < output.action_count; i++) {
        const cbor = output.action(i);
        target.push({
          kind: 'action',
          cbor: new Uint8Array(cbor),
        });
      }
    }
  }
}

// ─── Card Helpers ──────────────────────────────────────────────────

export const SUITS = ['clubs', 'diamonds', 'hearts', 'spades'];
export const RANKS = ['2', '3', '4', '5', '6', '7', '8', '9', 'T', 'J', 'Q', 'K', 'A'];

/**
 * Parse a card string like "As" → { suit: 'spades', rank: 'A' }
 * Matches the Rust card display format.
 */
export function parseCard(s) {
  if (!s || s.length < 2) return null;
  const rankChar = s[0];
  const suitChar = s[1];

  const suitMap = { c: 'clubs', d: 'diamonds', h: 'hearts', s: 'spades' };
  const rankMap = {
    '2': '2', '3': '3', '4': '4', '5': '5', '6': '6', '7': '7',
    '8': '8', '9': '9', 'T': '10', 'J': 'J', 'Q': 'Q', 'K': 'K', 'A': 'A',
  };

  const suit = suitMap[suitChar];
  const rank = rankMap[rankChar];
  if (!suit || !rank) return null;

  return { suit, rank };
}

/**
 * Create a standard 52-card deck (JS representation, for UI).
 */
export function createDeck() {
  const deck = [];
  for (const suit of SUITS) {
    for (const rank of RANKS) {
      deck.push({ suit, rank });
    }
  }
  return deck;
}

// ─── Legacy Compatibility ──────────────────────────────────────────

/**
 * Generate a seed for a player. In the WASM system, this is an
 * arbitrary byte array. For deterministic games, use a secret string.
 */
export function generateSeed(secret) {
  if (!secret) {
    const arr = new Uint8Array(32);
    crypto.getRandomValues(arr);
    return arr;
  }
  return new TextEncoder().encode(secret);
}

/**
 * Hash a seed commitment (BLAKE2b-like, for AT Protocol commit_seed).
 * Uses the WASM module's internal hashing when possible.
 */
export function hashSeed(seed) {
  // For now, use SubtleCrypto SHA-256 as a commitment hash.
  // The Rust system uses BLAKE2b-256 via blake2 crate.
  // In production, call the WASM module's blake2b function.
  const data = typeof seed === 'string'
    ? new TextEncoder().encode(seed)
    : seed;
  return crypto.subtle.digest('SHA-256', data);
}

// ─── No-op Stubs for Old API ───────────────────────────────────────

// These functions existed in mental-poker.js using NaCl.
// They're now replaced by the WASM agent. Kept as no-ops
// or redirects for gradual migration.

/** @deprecated Use CardcoreSession instead */
export function generateKeyPair() {
  console.warn('[cardcore-wasm] generateKeyPair is deprecated. Use CardcoreSession.');
  return { publicKey: '', secretKey: '' };
}

/** @deprecated Use CardcoreSession instead */
export function boxCard() {
  throw new Error('boxCard is deprecated. Use CardcoreSession with WASM agent.');
}

/** @deprecated Use CardcoreSession instead */
export function unboxCard() {
  throw new Error('unboxCard is deprecated. Use CardcoreSession with WASM agent.');
}

/** @deprecated Use CardcoreSession instead */
export function encryptLayer() {
  throw new Error('encryptLayer is deprecated. Use CardcoreSession with WASM agent.');
}

/** @deprecated Use CardcoreSession instead */
export function decryptLayer() {
  throw new Error('decryptLayer is deprecated. Use CardcoreSession with WASM agent.');
}

/** @deprecated Use CardcoreSession instead */
export function mentalShuffle() {
  throw new Error('mentalShuffle is deprecated. Use CardcoreSession with WASM agent.');
}

/** @deprecated Use CardcoreSession.getHoleCards() / getCommunityCards() instead */
export function dealCard() {
  throw new Error('dealCard is deprecated. Use CardcoreSession.getHoleCards().');
}

/** @deprecated Use CardcoreSession instead */
export function revealCard() {
  throw new Error('revealCard is deprecated. Use CardcoreSession with WASM agent.');
}
