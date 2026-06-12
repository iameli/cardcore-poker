/**
 * AT Protocol Publisher — re.cardco.* Lexicons
 *
 * Publishes table and action records using @atcute/client.
 * Schema definitions match lexicons/re/cardco/{poker,blackjack}/*.json.
 *
 * Lexicon IDs:
 *   re.cardco.poker.table      — establishes a poker game
 *   re.cardco.poker.action     — every poker action (commit, shuffle, lock, deal, bet, reveal)
 *   re.cardco.poker.defs       — poker union member types
 *   re.cardco.blackjack.table  — establishes a blackjack game
 *   re.cardco.blackjack.action — every blackjack action (commit, shuffle, lock, deal, wager, decision)
 *   re.cardco.blackjack.defs   — blackjack union member types
 */

import { Client } from "@atcute/client";

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

export const BLACKJACK_LEXICONS = {
  TABLE: "re.cardco.blackjack.table",
  ACTION: "re.cardco.blackjack.action",
};

export const BLACKJACK_ACTION_TYPES = {
  COMMIT_SEED: "re.cardco.blackjack.defs#commitSeed",
  SHUFFLE_DECK: "re.cardco.blackjack.defs#shuffleDeck",
  LOCK_DECK: "re.cardco.blackjack.defs#lockDeck",
  REVEAL_LOCK_KEY: "re.cardco.blackjack.defs#revealLockKey",
  WAGER: "re.cardco.blackjack.defs#wager",
  INSURANCE: "re.cardco.blackjack.defs#insurance",
  DECISION: "re.cardco.blackjack.defs#decision",
  VERIFY_SEED: "re.cardco.blackjack.defs#verifySeed",
};

/** Every table collection we know how to play. */
export const TABLE_COLLECTIONS = [LEXICONS.TABLE, BLACKJACK_LEXICONS.TABLE];

// ─── Record Builders ───────────────────────────────────────────────

export function buildTableRecord({
  collection = LEXICONS.TABLE,
  players,
  startingChips,
  smallBlind,
  minBet,
  startedAt,
  updatedAt,
}) {
  return {
    $type: collection,
    players,
    startingChips,
    ...(smallBlind !== undefined ? { smallBlind } : {}),
    ...(minBet !== undefined ? { minBet } : {}),
    ...(startedAt ? { startedAt } : {}),
    ...(updatedAt ? { updatedAt } : {}),
    createdAt: new Date().toISOString(),
  };
}

export function buildActionRecord({
  collection = LEXICONS.ACTION,
  tableRef,
  prevRef,
  seq,
  action,
}) {
  return {
    $type: collection,
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

// Blackjack action payload builders (crypto payloads come from the WASM
// agent already $type-tagged; these cover the player decisions).

export function buildWager(amount) {
  return { $type: BLACKJACK_ACTION_TYPES.WAGER, amount };
}

export function buildInsurance(take) {
  return { $type: BLACKJACK_ACTION_TYPES.INSURANCE, take };
}

export function buildDecision(move) {
  return { $type: BLACKJACK_ACTION_TYPES.DECISION, move };
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
   * Create a table record. Returns { uri, cid }. Pass a game's table
   * collection to create for another game (defaults to poker).
   */
  async createTable({ collection = LEXICONS.TABLE, players, startingChips, smallBlind, minBet }) {
    const record = buildTableRecord({ collection, players, startingChips, smallBlind, minBet });
    return this._createRecord(collection, record);
  }

  /**
   * Create an action record chained via prev. Pass a game's action
   * collection to create for another game (defaults to poker).
   */
  async createAction({ collection = LEXICONS.ACTION, tableRef, prevRef, seq, action }) {
    const record = buildActionRecord({ collection, tableRef, prevRef, seq, action });
    return this._createRecord(collection, record);
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
