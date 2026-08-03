/**
 * Live wiring for paid (settlement-valid) hands: connects the transport-
 * agnostic `ChainedSession` driver (./chained-driver.js) to the real AT
 * Protocol publisher and firehose.
 *
 * The driver decides the single global order and links; this module supplies:
 *   - `actionStrongRef` — the content-addressed strong ref `{ uri, cid }` of a
 *     published action record, computed from the record itself. Every chain
 *     `prev` must be a real strongRef so the atbloons node can re-fetch and
 *     verify it; the CID is the DASL CIDv1 (dag-cbor, sha-256) the PDS derives.
 *   - `makeChainedPublish` — wraps a `Publisher` (../transport.js) into the
 *     driver's `publish` contract: it already writes `prev`/global `seq`, and
 *     returns the record's strong ref for the next link.
 *   - `ChainedGame` — a thin façade owning one `ChainedSession`, feeding it the
 *     firehose stream (peer + own-echo action records) in the driver's shape.
 *
 * Dependency-light on purpose: only `@ipld/dag-cbor` (already a web dep). The
 * CID helper mirrors `transport.js#computeRecordCid` without pulling the atcute
 * client, so this module and its tests stay offline.
 */
import * as dagCbor from "@ipld/dag-cbor";
import { ChainedSession } from "./chained-driver.js";

export const ACTION_COLLECTION = "re.cardco.poker.action";

/** Action rkeys are `<tableTid>-<seq>`, seq zero-padded to 9 digits (as in transport.js). */
export function actionRkey(tableTid, seq) {
  if (seq >= 1_000_000_000) throw new Error(`action seq ${seq} exceeds rkey padding`);
  return `${tableTid}-${String(seq).padStart(9, "0")}`;
}

/** RFC 4648 base32, lowercase, no padding (multibase 'b'). */
function base32Lower(bytes) {
  const alphabet = "abcdefghijklmnopqrstuvwxyz234567";
  let out = "";
  let bits = 0;
  let value = 0;
  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      out += alphabet[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += alphabet[(value << (5 - bits)) & 31];
  return out;
}

/**
 * The DASL CID string of a record: CIDv1, dag-cbor codec, sha-256, base32-lower
 * — the same CID the PDS derives on commit. `record` must be in data-model form
 * (real Uint8Arrays, not `{ $bytes }` wrappers).
 */
export async function computeRecordCid(record) {
  const body = dagCbor.encode(record);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", body));
  const bytes = new Uint8Array(4 + digest.length);
  bytes.set([0x01, 0x71, 0x12, 0x20]);
  bytes.set(digest, 4);
  return "b" + base32Lower(bytes);
}

/**
 * The strong ref `{ uri, cid }` of an action record authored by `did` at global
 * `seq`. `record` is the full data-model action record (the object the firehose
 * decodes out of the CAR, with real Uint8Array byte fields).
 */
export async function actionStrongRef({ did, tableTid, seq, record }) {
  const cid = await computeRecordCid(record);
  return { uri: `at://${did}/${ACTION_COLLECTION}/${actionRkey(tableTid, seq)}`, cid };
}

/**
 * Wrap a `Publisher` into the `ChainedSession` publish contract. The publisher
 * already writes `prev`/global `seq` and returns `{ uri, cid }`; we forward the
 * running head and report the tip so the settle step can reference it.
 *
 * @param {object} opts
 * @param {import('../transport.js').Publisher} opts.publisher
 * @param {(ref: {uri, cid}, seq: number, actionCbor: Uint8Array) => void} [opts.onTip]
 */
export function makeChainedPublish({ publisher, onTip }) {
  return async ({ tableRef, prevRef, seq, tableTid, actionCbor }) => {
    const res = await publisher.publishAction({ tableRef, prevRef, seq, tableTid, actionCbor });
    const ref = { uri: res.uri, cid: res.cid };
    if (onTip) onTip(ref, seq, actionCbor);
    return ref;
  };
}

/**
 * A paid-hand game façade: one `ChainedSession` wired to a publisher and driven
 * by firehose action records. Unpaid play never constructs this.
 */
export class ChainedGame {
  /**
   * @param {object} opts
   * @param {object} opts.agent - chained WASM agent for this seat.
   * @param {string} opts.did
   * @param {{uri, cid}} opts.tableRef
   * @param {string} opts.tableTid
   * @param {import('../transport.js').Publisher} opts.publisher
   * @param {() => void} [opts.onUpdate]
   * @param {(ref: {uri, cid}, seq: number, actionCbor: Uint8Array) => void} [opts.onPublish]
   *   called after each of OUR actions is published, for logging/tip display.
   */
  constructor({ agent, did, tableRef, tableTid, publisher, onUpdate, onPublish }) {
    this.tableTid = tableTid;
    this.session = new ChainedSession({
      agent,
      did,
      tableRef,
      tableTid,
      onUpdate,
      publish: makeChainedPublish({ publisher, onTip: onPublish }),
    });
  }

  /** The WASM agent driving this seat (read-only state source for the UI). */
  get agent() {
    return this.session.agent;
  }

  /** Start the chain from the finalized table record. */
  async receiveTable(tableCbor) {
    await this.session.receiveTable(tableCbor);
  }

  /**
   * Deliver one firehose action. `did`/`seq` identify the global slot,
   * `actionCbor` is the inner action union the agent applies, and `record` is
   * the full data-model action record used to derive its strong ref for the
   * next `prev` link. Own-echo records (seq already passed) are ignored by the
   * session.
   */
  async deliverFirehoseAction(did, seq, actionCbor, record) {
    const ref = await actionStrongRef({ did, tableTid: this.tableTid, seq, record });
    await this.session.receiveChainAction(actionCbor, { did, seq, ref });
  }

  /** Provide a human betting decision when `needsBet` is set. */
  async submitBet(betString) {
    await this.session.submitBet(betString);
  }

  get needsBet() {
    return this.session.needsBet;
  }
  get betOptions() {
    return this.session.betOptions;
  }
  get isComplete() {
    return this.session.isComplete;
  }

  // ─── Rich read-only game state (agent-backed) ───────────────────────
  // These mirror PlayerSession's accessors so the same poker-table UI renders
  // a paid (chained) hand. They read straight from the WASM agent, which
  // advances identically for chained and unpaid play. Cards are returned raw
  // (short strings); the UI parses them, exactly as it does for unpaid play.
  get rawHoleCards() {
    try {
      return JSON.parse(this.agent.hole_cards());
    } catch {
      return [];
    }
  }
  get rawCommunityCards() {
    try {
      return JSON.parse(this.agent.community_cards());
    } catch {
      return [];
    }
  }
  get phase() {
    try {
      return this.agent.phase();
    } catch {
      return "Init";
    }
  }
  get gameState() {
    try {
      return JSON.parse(this.agent.game_state());
    } catch {
      return null;
    }
  }
  get waitingOn() {
    try {
      return JSON.parse(this.agent.waiting_on());
    } catch {
      return [];
    }
  }
  get lastHandResult() {
    try {
      const json = this.agent.last_hand_result();
      return json ? JSON.parse(json) : null;
    } catch {
      return null;
    }
  }
  get gameOver() {
    try {
      return this.agent.game_over();
    } catch {
      return false;
    }
  }
  get pendingCount() {
    return this.session.pendingCount;
  }

  /** Free the WASM agent. Safe to call once at teardown. */
  destroy() {
    try {
      this.session.agent?.free();
    } catch {
      // already freed / no-op in tests
    }
  }
  /**
   * The current GLOBAL chain tip strong ref — the last applied action, ours or
   * a peer's. When the hand is complete this is the terminal action the settle
   * intent references.
   */
  get tipRef() {
    return this.session.tipRef;
  }
}
