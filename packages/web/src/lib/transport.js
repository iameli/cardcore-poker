/**
 * AT Protocol transport for the poker protocol.
 *
 * Each player publishes their poker actions as `re.cardco.poker.action` records
 * to their own PDS. To learn what other players have done, we poll their
 * action collections via `com.atproto.repo.listRecords`. New records get
 * decoded, the inner action CBOR is reconstructed, and fed to the local
 * `WasmAgent`.
 *
 * Polling is a stand-in for `com.atproto.sync.subscribeRepos`. Latency is
 * ~`pollMs` (default 250ms). Swapping to firehose is a future optimization
 * — the public API of this module wouldn't change.
 */
import * as dagCbor from "@ipld/dag-cbor";
import { buildActionRecord, buildTableRecord, LEXICONS } from "./atproto-publisher.js";

const POLL_MS = 250;

function rkeyForSeq(tableTid, seq) {
  return `${tableTid}-${seq}`;
}

function tidFromTableUri(uri) {
  const parts = uri.split("/");
  return parts[parts.length - 1];
}

/**
 * Walk a value and turn `{ $bytes: base64 }` (the AT Protocol JSON wire format
 * for bytes) into Uint8Array, ready for DAG-CBOR encoding.
 *
 * Also handles numeric-keyed objects (`{0: x, 1: y, ...}`) — that's what
 * `JSON.stringify(uint8Array)` produces, and some round-trips through
 * @atcute's Client end up in this shape on the way back from listRecords.
 */
function rehydrateBytes(value) {
  if (value == null || typeof value !== "object") return value;
  if (ArrayBuffer.isView(value)) return value;
  if (Array.isArray(value)) return value.map(rehydrateBytes);
  if ("$bytes" in value && typeof value.$bytes === "string") {
    const bin = atob(value.$bytes);
    const arr = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
    return arr;
  }
  const keys = Object.keys(value);
  if (keys.length > 0 && keys.every((k) => /^\d+$/.test(k))) {
    const len = keys.length;
    const arr = new Uint8Array(len);
    for (let i = 0; i < len; i++) arr[i] = value[i];
    return arr;
  }
  const out = {};
  for (const [k, v] of Object.entries(value)) out[k] = rehydrateBytes(v);
  return out;
}

/**
 * Walk a value and turn Uint8Array values into `{ $bytes: base64 }`. This
 * is what AT Protocol's JSON wire format expects, and what @atcute's Client
 * passes through unchanged in JSON.stringify (which would otherwise emit a
 * numeric-keyed object that isn't a valid lexicon byte representation).
 */
function dehydrateBytes(value) {
  if (value == null || typeof value !== "object") return value;
  if (ArrayBuffer.isView(value)) {
    let bin = "";
    const u8 = value instanceof Uint8Array ? value : new Uint8Array(value.buffer);
    for (let i = 0; i < u8.length; i++) bin += String.fromCharCode(u8[i]);
    return { $bytes: btoa(bin) };
  }
  if (Array.isArray(value)) return value.map(dehydrateBytes);
  const out = {};
  for (const [k, v] of Object.entries(value)) out[k] = dehydrateBytes(v);
  return out;
}

/**
 * Encode the inner action object back to DAG-CBOR bytes for the WasmAgent.
 */
function encodeInnerAction(actionJson) {
  const rehydrated = rehydrateBytes(actionJson);
  return dagCbor.encode(rehydrated);
}

/**
 * Publishes records to the player's PDS via the @atcute Client we already
 * built during signin.
 */
export class Publisher {
  constructor({ client, did }) {
    this.client = client;
    this.did = did;
  }

  async createTable({ players, startingChips, smallBlind }) {
    const record = buildTableRecord({ players, startingChips, smallBlind });
    return this._createRecord(LEXICONS.TABLE, record);
  }

  /**
   * Publish a single poker action. Used for actions the local agent emits.
   * `actionCbor` is the raw CBOR the WasmAgent produced; we decode it to
   * reconstitute the inner action object as the lexicon expects.
   */
  async publishAction({ tableRef, prevRef, seq, tableTid, actionCbor }) {
    // Decode the WASM-emitted CBOR, then dehydrate Uint8Array fields to the
    // `{ $bytes: base64 }` JSON wire format the lexicon expects. Without this
    // step, @atcute's JSON.stringify turns Uint8Arrays into numeric-keyed
    // objects, which the Rust lexicon parser rejects on read.
    const innerAction = dehydrateBytes(dagCbor.decode(actionCbor));
    const record = buildActionRecord({
      tableRef,
      prevRef,
      seq,
      action: innerAction,
    });
    return this._createWithRkey(LEXICONS.ACTION, rkeyForSeq(tableTid, seq), record);
  }

  async _createRecord(collection, record) {
    const res = await this.client.post("com.atproto.repo.createRecord", {
      input: { repo: this.did, collection, record },
    });
    if (!res.ok) {
      throw new Error(
        `createRecord(${collection}) failed: ${res.status} ${JSON.stringify(res.data)}`,
      );
    }
    return { uri: res.data.uri, cid: res.data.cid };
  }

  async _createWithRkey(collection, rkey, record) {
    const res = await this.client.post("com.atproto.repo.putRecord", {
      input: { repo: this.did, collection, rkey, record },
    });
    if (!res.ok) {
      throw new Error(
        `putRecord(${collection}/${rkey}) failed: ${res.status} ${JSON.stringify(res.data)}`,
      );
    }
    return { uri: res.data.uri, cid: res.data.cid };
  }
}

/**
 * Polls every player's `re.cardco.poker.action` collection on the PDS and
 * emits new records (filtered to the active table) as they appear. Skips
 * records authored by `selfDid` since the local WasmAgent already knows
 * about its own actions.
 */
export class ActionPoller {
  /**
   * @param {object} opts
   * @param {object} opts.client - @atcute Client (any authenticated client; PDS lookups are public)
   * @param {string[]} opts.playerDids
   * @param {string} opts.tableUri - the canonical table AT URI (originator's repo)
   * @param {string} opts.selfDid - skip records authored by us
   * @param {(did: string, seq: number, actionCbor: Uint8Array) => void} opts.onAction
   * @param {number} [opts.pollMs]
   */
  constructor({ client, playerDids, tableUri, selfDid, onAction, pollMs = POLL_MS }) {
    this.client = client;
    this.playerDids = playerDids;
    this.tableUri = tableUri;
    this.tableTid = tidFromTableUri(tableUri);
    this.selfDid = selfDid;
    this.onAction = onAction;
    this.pollMs = pollMs;
    this.lastSeqByDid = new Map();
    this.timer = null;
    this.busy = false;
    this.stopped = false;
  }

  start() {
    if (this.timer) return;
    this.stopped = false;
    const tick = async () => {
      if (this.stopped) return;
      if (!this.busy) {
        this.busy = true;
        try {
          await this._pollOnce();
        } catch (e) {
          console.warn("[poller]", e);
        } finally {
          this.busy = false;
        }
      }
      this.timer = setTimeout(tick, this.pollMs);
    };
    tick();
  }

  stop() {
    this.stopped = true;
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
  }

  /**
   * Manually trigger a poll cycle. Useful right after publishing locally
   * so we catch up before waiting `pollMs`.
   */
  async pollNow() {
    if (this.busy) return;
    this.busy = true;
    try {
      await this._pollOnce();
    } finally {
      this.busy = false;
    }
  }

  async _pollOnce() {
    // Gather all new records across all players, then feed in chronological order.
    const fresh = [];
    for (const did of this.playerDids) {
      if (did === this.selfDid) continue;
      const lastSeq = this.lastSeqByDid.get(did) ?? -1;
      const records = await this._listSince(did, lastSeq);
      for (const r of records) fresh.push({ did, ...r });
    }
    fresh.sort((a, b) => {
      if (a.value.createdAt !== b.value.createdAt) {
        return a.value.createdAt < b.value.createdAt ? -1 : 1;
      }
      return a.value.seq - b.value.seq;
    });
    for (const item of fresh) {
      const { did, value } = item;
      if (value?.table?.uri !== this.tableUri) continue;
      const cbor = encodeInnerAction(value.action);
      try {
        this.onAction(did, value.seq, cbor);
      } catch (e) {
        // Out-of-order or already-applied — agents log internally.
        console.warn(`[poller] feed ${did}@seq=${value.seq} threw:`, e?.message || e);
      }
      this.lastSeqByDid.set(did, Math.max(this.lastSeqByDid.get(did) ?? -1, value.seq));
    }
  }

  async _listSince(did, sinceSeq) {
    // listRecords returns rkeys in reverse-chronological order by default; we
    // want forward. Pull a page (~50) and filter client-side.
    const res = await this.client.get("com.atproto.repo.listRecords", {
      params: {
        repo: did,
        collection: LEXICONS.ACTION,
        limit: 100,
        reverse: true, // oldest first
      },
    });
    if (!res.ok) return [];
    const records = res.data.records || [];
    return records.filter((r) => {
      const seq = r.value?.seq;
      return typeof seq === "number" && seq > sinceSeq;
    });
  }
}
