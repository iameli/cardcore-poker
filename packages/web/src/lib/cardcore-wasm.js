/**
 * Cardcore WASM Bridge
 *
 * Replaces the NaCl-based mental poker with the Rust WASM module
 * using Ristretto255 commutative encryption and the two-phase
 * shuffle+lock protocol described in the README.
 */

import init, { WasmAgent, simulate_game } from "../../../../pkg/cardcore_poker.js";
import * as dagCbor from "@ipld/dag-cbor";

// ─── WASM Init ─────────────────────────────────────────────────────

let _wasmReady = false;
let _wasmInitPromise = null;

export async function initWasm() {
  if (_wasmReady) return;
  if (_wasmInitPromise) return _wasmInitPromise;
  _wasmInitPromise = init().then(() => {
    _wasmReady = true;
    console.log("[cardcore-wasm] WASM module initialized");
  });
  return _wasmInitPromise;
}

// ─── Player Agent ──────────────────────────────────────────────────

export function createAgent(did, seed) {
  if (typeof seed === "string") {
    seed = new TextEncoder().encode(seed);
  }
  if (!(seed instanceof Uint8Array)) {
    throw new Error("seed must be Uint8Array or string");
  }
  return new WasmAgent(did, seed);
}

export function encodeRecord(record) {
  return dagCbor.encode(record);
}

export function decodeRecord(cbor) {
  return dagCbor.decode(cbor);
}

// ─── Multi-Agent Session ───────────────────────────────────────────

/**
 * Manages WasmAgent instances for all players.
 * Used for simulations and testing. For real multiplayer,
 * use PlayerSession from game-session.js.
 */
export class CardcoreSession {
  constructor({ playerDids, seeds, startingChips = 1000, smallBlind = 10 }) {
    this.agents = {};
    this.playerOrder = playerDids;
    this.startingChips = BigInt(startingChips);
    this.smallBlind = BigInt(smallBlind);

    for (let i = 0; i < playerDids.length; i++) {
      const did = playerDids[i];
      const seed = seeds[i] || new Uint8Array(32);
      this.agents[did] = createAgent(did, seed);
    }
  }

  startTable() {
    const tableCbor = encodeRecord({
      $type: "re.cardco.poker.table",
      players: this.playerOrder,
      startingChips: Number(this.startingChips),
      smallBlind: Number(this.smallBlind),
      createdAt: new Date().toISOString(),
    });

    const allActions = [];
    for (const did of this.playerOrder) {
      const output = this.agents[did].receive_table(tableCbor);
      this._collect(output, allActions);
    }
    return allActions;
  }

  processAction(actionCbor) {
    const allActions = [];
    for (const did of this.playerOrder) {
      const output = this.agents[did].receive_action(actionCbor);
      this._collect(output, allActions);
    }
    return allActions;
  }

  submitBet(did, betAction) {
    const agent = this.agents[did];
    if (!agent) throw new Error(`Unknown player: ${did}`);
    return this._collect(agent.bet(betAction), []);
  }

  getHoleCards(did) {
    const agent = this.agents[did];
    if (!agent) return [];
    return JSON.parse(agent.hole_cards());
  }

  getCommunityCards(did) {
    const agent = this.agents[did];
    if (!agent) return [];
    return JSON.parse(agent.community_cards());
  }

  checkStatus(did) {
    const agent = this.agents[did];
    if (!agent) return { kind: "waiting", options: [] };
    const output = agent.check_status();
    return {
      kind: output.kind,
      options: output.kind === "need_bet" ? JSON.parse(output.bet_options) : [],
    };
  }

  simulate(numPlayers, chips, blind, strategy = "random", seed = 42n) {
    return simulate_game(numPlayers, chips, blind, strategy, seed);
  }

  _collect(output, target) {
    const actions = [];
    if (output.kind === "actions") {
      for (let i = 0; i < output.action_count; i++) {
        const cbor = new Uint8Array(output.action(i));
        target.push({ kind: "action", cbor });
        actions.push(cbor);
      }
    }
    return actions;
  }
}

// ─── Card Helpers ──────────────────────────────────────────────────

export const SUITS = ["clubs", "diamonds", "hearts", "spades"];
export const RANKS = ["2", "3", "4", "5", "6", "7", "8", "9", "T", "J", "Q", "K", "A"];

export function parseCard(s) {
  if (!s || s.length < 2) return null;
  const suitMap = { c: "clubs", d: "diamonds", h: "hearts", s: "spades" };
  const rankMap = {
    2: "2",
    3: "3",
    4: "4",
    5: "5",
    6: "6",
    7: "7",
    8: "8",
    9: "9",
    T: "10",
    J: "J",
    Q: "Q",
    K: "K",
    A: "A",
  };
  const suit = suitMap[s[1]];
  const rank = rankMap[s[0]];
  if (!suit || !rank) return null;
  return { suit, rank };
}

export function createDeck() {
  const deck = [];
  for (const suit of SUITS) {
    for (const rank of RANKS) {
      deck.push({ suit, rank });
    }
  }
  return deck;
}

export function generateSeed(secret) {
  if (!secret) {
    const arr = new Uint8Array(32);
    crypto.getRandomValues(arr);
    return arr;
  }
  return new TextEncoder().encode(secret);
}

export function hashSeed(seed) {
  const data = typeof seed === "string" ? new TextEncoder().encode(seed) : seed;
  return crypto.subtle.digest("SHA-256", data);
}
