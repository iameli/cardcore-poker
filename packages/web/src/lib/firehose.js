/**
 * Firehose subscriber for `com.atproto.sync.subscribeRepos`.
 *
 * Each peer in a poker hand has their action records on their own PDS. We
 * open one WebSocket per unique PDS, decode the binary CBOR frame stream,
 * filter for #commit events from the player DIDs we care about, dig the
 * action records out of the CAR-encoded `blocks` payload, and emit the
 * inner action CBOR for the local WasmAgent.
 *
 * On startup we also do a one-shot listRecords backfill to catch anything
 * that was published before we subscribed (e.g. a dealer who published their
 * commitSeed before the joiner navigated to the game). Events are
 * deduplicated by (did, seq) so backfill + live can race safely.
 */
import { decodeMultiple } from "cbor-x";
import { CarReader } from "@ipld/car";
import * as dagCbor from "@ipld/dag-cbor";
import { pdsForDid } from "./atproto.js";
import { LEXICONS } from "./atproto-publisher.js";

const ACTION_PATH_PREFIX = `${LEXICONS.ACTION}/`;

function decodeFrame(bytes) {
  const values = [];
  decodeMultiple(bytes, (v) => values.push(v));
  if (values.length < 2) return null;
  const [header, body] = values;
  if (header?.op !== 1) return null;
  return { t: header.t, ...body };
}

/**
 * Walk every block in the CAR and yield decoded poker.action records.
 * We don't bother matching the op CIDs — there are typically only a handful
 * of blocks per commit and decoding each is cheap. Anything that doesn't
 * decode as a re.cardco.poker.action record is silently skipped.
 */
async function* extractActionRecords(carBytes) {
  const reader = await CarReader.fromBytes(carBytes);
  for await (const block of reader.blocks()) {
    let record;
    try {
      record = dagCbor.decode(block.bytes);
    } catch {
      continue;
    }
    if (record?.$type === LEXICONS.ACTION) yield record;
  }
}

function pdsToWsBase(pdsUri) {
  // http://x → ws://x ; https://x → wss://x
  return pdsUri.replace(/^http/, "ws");
}

export class FirehoseSubscriber {
  /**
   * @param {object} opts
   * @param {string[]} opts.peerDids - peer DIDs to listen for (excludes self)
   * @param {string} opts.tableUri - table AT URI (for filtering action records)
   * @param {string} opts.ownPdsUri - the local user's PDS, used as the dev fallback
   * @param {(did: string, seq: number, actionCbor: Uint8Array) => void} opts.onAction
   */
  constructor({ peerDids, tableUri, ownPdsUri, onAction }) {
    this.peerDids = peerDids;
    this.tableUri = tableUri;
    this.ownPdsUri = ownPdsUri;
    this.onAction = onAction;
    this.seen = new Set(); // `${did}:${seq}` keys
    this.sockets = []; // { ws, dids: Set }
    this.stopped = false;
    this.reconnectDelays = new Map(); // pdsUri → ms
    this.cursorByPds = new Map(); // pdsUri → last firehose seq seen
    this.pdsByDid = new Map(); // did → pdsUri
  }

  async start() {
    // Resolve every peer DID → PDS up front. Group peers by PDS so we open
    // one socket per host.
    const byPds = new Map(); // pdsUri → did[]
    await Promise.all(
      this.peerDids.map(async (did) => {
        try {
          const pds = await pdsForDid(did, this.ownPdsUri);
          this.pdsByDid.set(did, pds);
          if (!byPds.has(pds)) byPds.set(pds, []);
          byPds.get(pds).push(did);
        } catch (e) {
          console.warn(`[firehose] could not resolve PDS for ${did}:`, e?.message || e);
        }
      }),
    );

    // Backfill from each peer's PDS via listRecords (the canonical PDS for
    // that peer, NOT the local user's). Live events from before we
    // subscribed would otherwise be missed.
    await Promise.all(this.peerDids.map((did) => this._backfill(did)));

    // Open one firehose subscription per unique PDS.
    for (const [pds, dids] of byPds) {
      this._openSocket(pds, new Set(dids));
    }
  }

  stop() {
    this.stopped = true;
    for (const { ws } of this.sockets) {
      try {
        ws.close();
      } catch {}
    }
    this.sockets = [];
  }

  async _backfill(did) {
    const pds = this.pdsByDid.get(did);
    if (!pds) return;
    try {
      const url =
        `${pds}/xrpc/com.atproto.repo.listRecords` +
        `?repo=${encodeURIComponent(did)}` +
        `&collection=${encodeURIComponent(LEXICONS.ACTION)}` +
        `&limit=100&reverse=true`;
      const res = await fetch(url);
      if (!res.ok) return;
      const data = await res.json();
      const records = data.records || [];
      for (const r of records) {
        if (r.value?.table?.uri !== this.tableUri) continue;
        const seq = r.value.seq;
        this._dispatch(did, seq, () => this._actionFromJsonRecord(r.value));
      }
    } catch (e) {
      console.warn(`[firehose] backfill ${did} failed:`, e?.message || e);
    }
  }

  _openSocket(pdsUri, dids) {
    if (this.stopped) return;
    // On reconnect, resume from the last firehose seq we saw on this PDS so
    // we don't miss anything that happened during the gap.
    const cursor = this.cursorByPds.get(pdsUri);
    const url =
      `${pdsToWsBase(pdsUri)}/xrpc/com.atproto.sync.subscribeRepos` +
      (cursor != null ? `?cursor=${cursor}` : "");
    const ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";
    const slot = { ws, pdsUri, dids };
    this.sockets.push(slot);

    ws.addEventListener("open", () => {
      this.reconnectDelays.set(pdsUri, 0);
    });

    ws.addEventListener("message", async (ev) => {
      try {
        const frame = decodeFrame(new Uint8Array(ev.data));
        if (!frame) return;
        // Track the firehose-level seq from EVERY frame so reconnects can
        // resume — even from frames we don't otherwise care about.
        if (typeof frame.seq === "number") {
          const prev = this.cursorByPds.get(pdsUri) ?? -1;
          if (frame.seq > prev) this.cursorByPds.set(pdsUri, frame.seq);
        }
        if (frame.t !== "#commit") return;
        if (!dids.has(frame.repo)) return;
        if (!frame.blocks) return;

        // Skip if all the ops in this commit are deletes; we only care about creates.
        const hasActionOp = (frame.ops || []).some((op) => op.path?.startsWith(ACTION_PATH_PREFIX));
        if (!hasActionOp) return;

        for await (const record of extractActionRecords(new Uint8Array(frame.blocks))) {
          if (record.table?.uri !== this.tableUri) continue;
          const seq = record.seq;
          this._dispatch(frame.repo, seq, () => this._cborFromRecord(record));
        }
      } catch (e) {
        console.warn("[firehose] frame error:", e?.message || e);
      }
    });

    ws.addEventListener("close", () => {
      if (this.stopped) return;
      // Exponential backoff: 1s, 2s, 4s, ... capped at 30s.
      const prev = this.reconnectDelays.get(pdsUri) ?? 0;
      const next = prev === 0 ? 1000 : Math.min(prev * 2, 30_000);
      this.reconnectDelays.set(pdsUri, next);
      this.sockets = this.sockets.filter((s) => s !== slot);
      setTimeout(() => this._openSocket(pdsUri, dids), next);
    });

    ws.addEventListener("error", () => {
      // close handler will retry.
    });
  }

  _dispatch(did, seq, makeCbor) {
    const key = `${did}:${seq}`;
    if (this.seen.has(key)) return;
    this.seen.add(key);
    try {
      const cbor = makeCbor();
      this.onAction(did, seq, cbor);
    } catch (e) {
      console.warn(`[firehose] dispatch ${key} threw:`, e?.message || e);
    }
  }

  /** From a record decoded out of the CAR (already DAG-CBOR-shaped). */
  _cborFromRecord(record) {
    return dagCbor.encode(record.action);
  }

  /** From a JSON record returned by listRecords (bytes are wire-format). */
  _actionFromJsonRecord(value) {
    const action = rehydrateBytes(value.action);
    return dagCbor.encode(action);
  }
}

// ─── Local helpers ───────────────────────────────────────────────────

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
