/**
 * Firehose subscriber for `com.atproto.sync.subscribeRepos`.
 *
 * Two modes:
 *
 *  1. **Filtered firehose service** (prod) — when `VITE_FIREHOSE_URL` is set,
 *     we open a single WebSocket to that endpoint and pass every peer DID as
 *     a `wantedDids` query parameter. The service does the filtering for us
 *     so we only receive commits from the players in this hand. One socket
 *     covers any number of peers regardless of which PDS they're on.
 *
 *  2. **Per-PDS** (dev fallback) — when no firehose service is configured,
 *     we resolve each peer's PDS via the DID document (or the dev shortcut
 *     to the local PDS) and open one socket per unique host. We filter
 *     client-side. Wasteful on the public network; fine for the local dev
 *     PDS since it only knows our demo accounts.
 *
 * On startup we also do a one-shot listRecords backfill (always against each
 * peer's authoritative PDS) to catch anything published before we
 * subscribed. Events are deduplicated by (did, seq) so backfill + live can
 * race safely.
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

function originToWs(uri) {
  // http://x → ws://x ; https://x → wss://x ; ws[s]://x stays as-is.
  if (uri.startsWith("ws://") || uri.startsWith("wss://")) return uri;
  return uri.replace(/^http/, "ws");
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
    this.sockets = []; // { ws, key, dids: Set }
    this.stopped = false;
    this.reconnectDelays = new Map(); // socket key → ms
    this.cursorBySocket = new Map(); // socket key → last firehose seq seen
    this.pdsByDid = new Map(); // did → pdsUri (always populated for backfill)
  }

  async start() {
    // Resolve every peer DID → PDS up front. Needed for backfill regardless
    // of which firehose mode we use.
    await Promise.all(
      this.peerDids.map(async (did) => {
        try {
          const pds = await pdsForDid(did, this.ownPdsUri);
          this.pdsByDid.set(did, pds);
        } catch (e) {
          console.warn(`[firehose] could not resolve PDS for ${did}:`, e?.message || e);
        }
      }),
    );

    // Backfill from each peer's PDS via listRecords. Live events from before
    // we subscribed would otherwise be missed.
    await Promise.all(this.peerDids.map((did) => this._backfill(did)));

    const filteredFirehose = import.meta.env.VITE_FIREHOSE_URL;
    if (filteredFirehose) {
      // Single connection to the filtered firehose service.
      const dids = new Set(this.peerDids);
      this._openSocket({
        key: "filtered",
        wsBase: originToWs(filteredFirehose),
        dids,
        wantedDids: this.peerDids,
      });
    } else {
      // Fallback: one socket per unique PDS.
      const byPds = new Map(); // pdsUri → did[]
      for (const [did, pds] of this.pdsByDid) {
        if (!byPds.has(pds)) byPds.set(pds, []);
        byPds.get(pds).push(did);
      }
      for (const [pds, dids] of byPds) {
        this._openSocket({
          key: pds,
          wsBase: originToWs(pds),
          dids: new Set(dids),
        });
      }
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

  _openSocket({ key, wsBase, dids, wantedDids }) {
    if (this.stopped) return;
    const params = new URLSearchParams();
    if (wantedDids) {
      for (const did of wantedDids) params.append("wantedDids", did);
    }
    const cursor = this.cursorBySocket.get(key);
    if (cursor != null) params.set("cursor", String(cursor));
    const qs = params.toString();
    const url = `${wsBase}/xrpc/com.atproto.sync.subscribeRepos${qs ? `?${qs}` : ""}`;

    const ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";
    const slot = { ws, key, dids, wantedDids };
    this.sockets.push(slot);

    ws.addEventListener("open", () => {
      this.reconnectDelays.set(key, 0);
    });

    ws.addEventListener("message", async (ev) => {
      try {
        const frame = decodeFrame(new Uint8Array(ev.data));
        if (!frame) return;
        // Track the firehose-level seq from EVERY frame so reconnects can
        // resume — even from frames we don't otherwise care about.
        if (typeof frame.seq === "number") {
          const prev = this.cursorBySocket.get(key) ?? -1;
          if (frame.seq > prev) this.cursorBySocket.set(key, frame.seq);
        }
        if (frame.t !== "#commit") return;
        if (!dids.has(frame.repo)) return;
        if (!frame.blocks) return;

        // Skip if no ops touch our action collection.
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
      const prev = this.reconnectDelays.get(key) ?? 0;
      const next = prev === 0 ? 1000 : Math.min(prev * 2, 30_000);
      this.reconnectDelays.set(key, next);
      this.sockets = this.sockets.filter((s) => s !== slot);
      setTimeout(() => this._openSocket({ key, wsBase, dids, wantedDids }), next);
    });

    ws.addEventListener("error", () => {
      // close handler will retry.
    });
  }

  _dispatch(did, seq, makeCbor) {
    const dedupKey = `${did}:${seq}`;
    if (this.seen.has(dedupKey)) return;
    this.seen.add(dedupKey);
    try {
      const cbor = makeCbor();
      this.onAction(did, seq, cbor);
    } catch (e) {
      console.warn(`[firehose] dispatch ${dedupKey} threw:`, e?.message || e);
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
