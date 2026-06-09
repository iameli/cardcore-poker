/**
 * AT Protocol Publisher — re.cardco.poker Lexicons
 *
 * Publishes table and action records using @atcute/client.
 * Schema definitions match lexicons/re/cardco/poker/*.json.
 *
 * Lexicon IDs:
 *   re.cardco.poker.table  — establishes a game
 *   re.cardco.poker.action  — every game action (commit, shuffle, lock, deal, bet, reveal)
 *   re.cardco.poker.defs    — union member types
 */

import { Client } from "@atcute/client";
import { encodeRecord } from "./cardcore-wasm.js";

// ─── Lexicon Constants ─────────────────────────────────────────────

export const LEXICONS = {
  TABLE: "re.cardco.poker.table",
  ACTION: "re.cardco.poker.action",
};

export const ACTION_TYPES = {
  COMMIT_SEED: "re.cardco.poker.defs#commitSeed",
  SHUFFLE_DECK: "re.cardco.poker.defs#shuffleDeck",
  LOCK_DECK: "re.cardco.poker.defs#lockDeck",
  REVEAL_LOCK_KEY: "re.cardco.poker.defs#revealLockKey",
  BET: "re.cardco.poker.defs#bet",
  REVEAL_HAND: "re.cardco.poker.defs#revealHand",
  VERIFY_SEED: "re.cardco.poker.defs#verifySeed",
};

// ─── Record Builders ───────────────────────────────────────────────

export function buildTableRecord({ players, startingChips, smallBlind, startedAt }) {
  return {
    $type: LEXICONS.TABLE,
    players,
    startingChips,
    smallBlind,
    ...(startedAt ? { startedAt } : {}),
    createdAt: new Date().toISOString(),
  };
}

export function buildActionRecord({ tableRef, prevRef, seq, action }) {
  return {
    $type: LEXICONS.ACTION,
    table: tableRef,
    seq,
    action,
    createdAt: new Date().toISOString(),
    ...(prevRef ? { prev: prevRef } : {}),
  };
}

export function buildCommitSeed(commitment) {
  return { $type: ACTION_TYPES.COMMIT_SEED, commitment };
}

export function buildShuffleDeck(deck) {
  return { $type: ACTION_TYPES.SHUFFLE_DECK, deck };
}

export function buildLockDeck(deck) {
  return { $type: ACTION_TYPES.LOCK_DECK, deck };
}

export function buildRevealLockKey(deckPosition, scalar) {
  return { $type: ACTION_TYPES.REVEAL_LOCK_KEY, deckPosition, scalar };
}

export function buildBet(action, amount) {
  const bet = { $type: ACTION_TYPES.BET, action };
  if (amount !== undefined) bet.amount = amount;
  return bet;
}

export function buildRevealHand(reveals) {
  return {
    $type: ACTION_TYPES.REVEAL_HAND,
    reveals: reveals.map((r) => ({ deckPosition: r.deckPosition, scalar: r.scalar })),
  };
}

export function buildVerifySeed(seed) {
  return { $type: ACTION_TYPES.VERIFY_SEED, seed };
}

// ─── AT Protocol Publisher ─────────────────────────────────────────

/**
 * Publishes records to an AT Protocol PDS via @atcute/client.
 *
 * The authenticated fetch handler comes from @atcute/oauth-browser-client
 * session. Example:
 *
 *   import { getSession } from '@atcute/oauth-browser-client';
 *   const session = await getSession(did);
 *   const publisher = new ATPublisher({
 *     handler: session.fetchHandler,
 *     did: session.info.sub,
 *   });
 */
export class ATPublisher {
  /**
   * @param {object} opts
   * @param {import('@atcute/client').FetchHandler} opts.handler - authenticated fetch from atcute session
   * @param {string} opts.did - the authenticated user's DID
   */
  constructor({ handler, did }) {
    this.did = did;
    this.client = new Client({ handler });
  }

  /**
   * Create a table record. Returns { uri, cid }.
   */
  async createTable({ players, startingChips, smallBlind }) {
    const record = buildTableRecord({ players, startingChips, smallBlind });
    return this._createRecord(LEXICONS.TABLE, record);
  }

  /**
   * Create an action record chained via prev.
   */
  async createAction({ tableRef, prevRef, seq, action }) {
    const record = buildActionRecord({ tableRef, prevRef, seq, action });
    return this._createRecord(LEXICONS.ACTION, record);
  }

  /**
   * Get a record by AT URI (at://did/collection/rkey).
   */
  async getRecord(uri) {
    const parts = uri.replace("at://", "").split("/");
    const [repo, collection, rkey] = [parts[0], parts[1], parts[2]];

    const res = await this.client.get("com.atproto.repo.getRecord", {
      params: { repo, collection, rkey },
    });
    if (!res.ok) throw new Error(`getRecord failed: ${res.status}`);
    return res.data;
  }

  /**
   * List records in a collection for the authenticated DID.
   */
  async listRecords({ collection, limit = 50, cursor }) {
    const params = { repo: this.did, collection, limit };
    if (cursor) params.cursor = cursor;

    const res = await this.client.get("com.atproto.repo.listRecords", { params });
    if (!res.ok) throw new Error(`listRecords failed: ${res.status}`);
    return res.data;
  }

  async _createRecord(collection, record) {
    const res = await this.client.post("com.atproto.repo.createRecord", {
      params: { repo: this.did, collection },
      input: { repo: this.did, collection, record },
    });

    if (!res.ok) {
      throw new Error(`createRecord failed: ${res.status} - ${JSON.stringify(res.data)}`);
    }

    return { uri: res.data.uri, cid: res.data.cid };
  }
}

// ─── Game Action Chain ─────────────────────────────────────────────

/**
 * Manages the action chain — table ref, prev ref, sequence numbers
 * per re.cardco.poker.action lexicon.
 */
export class ActionChain {
  constructor() {
    this.tableRef = null;
    this.prevRef = null;
    this.seq = 0;
    this.actions = [];
  }

  startHand(tableUri, tableCid) {
    this.tableRef = { uri: tableUri, cid: tableCid };
    this.prevRef = null;
    this.seq = 0;
    this.actions = [];
  }

  pushAction(actionUri, actionCid) {
    const ref = { uri: actionUri, cid: actionCid };
    this.actions.push({ seq: this.seq, ref });
    this.prevRef = ref;
    this.seq++;
  }

  get currentRefs() {
    return {
      tableRef: this.tableRef,
      prevRef: this.prevRef,
      seq: this.seq,
    };
  }
}
