/**
 * Game Session — WASM-backed multiplayer poker session.
 *
 * Wraps the Rust WasmAgent for WebSocket-based multiplayer.
 * Each player runs their own WasmAgent. Actions flow through
 * WebSocket as DAG-CBOR encoded re.cardco.poker.action records.
 *
 * Protocol:
 * 1. Table established → each agent receives table CBOR
 * 2. Agents auto-produce: commit → shuffle → lock → reveal
 * 3. Cards are queried from the agent (hole_cards, community_cards)
 * 4. Betting: agent.check_status() → if need_bet → agent.bet(action)
 * 5. All actions are broadcast via WebSocket to other players
 */

import { createAgent, encodeRecord, decodeRecord, parseCard } from './cardcore-wasm.js';

/**
 * Manages a single player's WASM-backed poker session.
 * Communicates with other players via a send callback (WebSocket).
 */
export class PlayerSession {
  /**
   * @param {object} opts
   * @param {string} opts.did - player DID
   * @param {Uint8Array|string} opts.seed - secret seed
   * @param {function} opts.send - callback to broadcast CBOR actions
   */
  constructor({ did, seed, send }) {
    this.did = did;
    this.agent = createAgent(did, seed);
    this.send = send;
    this.seat = -1;
    this.ready = false;
    this._phase = 'init';
    this._holeCards = [];
    this._communityCards = [];
    this._betOptions = [];
    this._needsBet = false;
  }

  /** Feed the table record. Returns actions to broadcast. */
  receiveTable(tableCbor) {
    console.log('[PlayerSession] receiveTable DID=' + this.did.slice(-8));

    const output = this.agent.receive_table(tableCbor);
    return this._processOutput(output);
  }

  /** Feed an action from another player. Returns actions to broadcast. */
  receiveAction(actionCbor) {
    console.log('[PlayerSession] receiveAction DID=' + this.did.slice(-8), 'len=' + actionCbor.length);

    const output = this.agent.receive_action(actionCbor);
    return this._processOutput(output);
  }

  /** Submit a bet decision. Returns actions to broadcast. */
  bet(action) {
    const output = this.agent.bet(action);
    return this._processOutput(output);
  }

  /** Check if we need to make a bet. */
  checkStatus() {
    const output = this.agent.check_status();
    return this._processOutput(output);
  }

  /** Get our hole cards as parsed objects. */
  get holeCards() {
    try {
      const raw = this.agent.hole_cards();
      if (!raw || raw === '[]') return [];
      return JSON.parse(raw).map(parseCard).filter(Boolean);
    } catch {
      return this._holeCards;
    }
  }

  /** Get community cards as parsed objects. */
  get communityCards() {
    try {
      const raw = this.agent.community_cards();
      if (!raw || raw === '[]') return [];
      return JSON.parse(raw).map(parseCard).filter(Boolean);
    } catch {
      return this._communityCards;
    }
  }

  /** Current phase from the agent. */
  get phase() {
    return this._phase;
  }

  /** Whether the agent needs a bet decision. */
  get needsBet() {
    return this._needsBet;
  }

  /** Available bet options. */

  /** Whether the game is complete (Phase::Complete). */
  get isComplete() {
    try { return this.agent.phase() === "Complete"; } catch { return false; }
  }


  get betOptions() {
    return this._betOptions;
  }

  _processOutput(output) {
    console.log('[PlayerSession] output kind=' + output.kind, 'n=' + (output.action_count || 0));

    const actions = [];
    if (output.kind === 'actions') {
      for (let i = 0; i < output.action_count; i++) {
        const cbor = new Uint8Array(output.action(i));
        actions.push(cbor);
        // Broadcast each action
        if (this.send) {
          this.send(cbor);
        }
      }
      // Refresh card state after actions
      this._refreshCards();
      this._phase = 'playing';
      this._needsBet = false;
    } else if (output.kind === 'need_bet') {
      this._needsBet = true;
      this._phase = 'betting';
      try {
        this._betOptions = JSON.parse(output.bet_options);
      } catch {
        this._betOptions = [];
      }
    } else {
      this._phase = 'waiting';
      this._needsBet = false;
    }
    return actions;
  }

  _refreshCards() {
    try { this._holeCards = JSON.parse(this.agent.hole_cards()); } catch {}
    try { this._communityCards = JSON.parse(this.agent.community_cards()); } catch {}
  }

  /** Clean up. */
  destroy() {
    if (this.agent) {
      try { this.agent.free(); } catch {}
      this.agent = null;
    }
  }
}

/**
 * Build a table record CBOR for the WasmAgent.
 */
export function buildTableCbor({ players, startingChips, smallBlind }) {
  return encodeRecord({
    $type: 're.cardco.poker.table',
    players,
    startingChips,
    smallBlind,
    createdAt: new Date().toISOString(),
  });
}

/**
 * Generate a random 32-byte seed.
 */
export function generateSeed() {
  const seed = new Uint8Array(32);
  crypto.getRandomValues(seed);
  return seed;
}
