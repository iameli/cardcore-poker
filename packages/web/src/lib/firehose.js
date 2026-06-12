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
 * Ordering matters: the live socket is opened FIRST, and a listRecords
 * backfill (always against each peer's authoritative PDS) runs once it's
 * connected. The reverse order has a delivery gap — a record published after
 * the backfill snapshot but before the subscription went live is never
 * delivered, and game start sits squarely in it (every peer publishes
 * commitSeed within seconds of the table record). Events are deduplicated by
 * (did, seq) so backfill + live can race safely.
 *
 * Frames can also be lost mid-game without the socket closing (flaky
 * connections, relay hiccups) — and the firehose never redelivers. As a
 * safety net, whenever the stream has been quiet for a while we re-sweep the
 * PDSes; a sweep that finds nothing new is cheap and side-effect-free.
 */
import { decodeMultiple } from "cbor-x";
import { CarReader } from "@ipld/car";
import * as dagCbor from "@ipld/dag-cbor";
import { pdsForDid } from "./atproto.js";
import { LEXICONS } from "./atproto-publisher.js";

// Re-sweep the PDSes when no action has arrived for this long. A dropped
// frame would otherwise stall the protocol forever — the firehose never
// redelivers. Idle sweeps during a long betting think are small listRecords
// reads, an acceptable price for unsticking a game within ~10s.
const QUIET_RESWEEP_MS = 8_000;
const RESWEEP_POLL_MS = 4_000;
// How long start() waits for the live socket before backfilling anyway — a
// relay that can't connect must degrade to polling, not block startup.
const SOCKET_OPEN_TIMEOUT_MS = 3_000;

function decodeFrame(bytes) {
  const values = [];
  decodeMultiple(bytes, (v) => values.push(v));
  if (values.length < 2) return null;
  const [header, body] = values;
  if (header?.op !== 1) return null;
  return { t: header.t, ...body };
}

/**
 * Walk every block in the CAR and yield decoded action records for the
 * given collection. We don't bother matching the op CIDs — there are
 * typically only a handful of blocks per commit and decoding each is cheap.
 * Anything that doesn't decode as an action record is silently skipped.
 */
async function* extractActionRecords(carBytes, actionCollection) {
  const reader = await CarReader.fromBytes(carBytes);
  for await (const block of reader.blocks()) {
    let record;
    try {
      record = dagCbor.decode(block.bytes);
    } catch {
      continue;
    }
    if (record?.$type === actionCollection) yield record;
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
   * @param {string} [opts.actionCollection] - action record collection (defaults to poker)
   * @param {(did: string, seq: number, actionCbor: Uint8Array) => void} opts.onAction
   */
  constructor({ peerDids, tableUri, ownPdsUri, onAction, actionCollection = LEXICONS.ACTION }) {
    this.peerDids = peerDids;
    this.tableUri = tableUri;
    this.ownPdsUri = ownPdsUri;
    this.onAction = onAction;
    this.actionCollection = actionCollection;
    this.actionPathPrefix = `${actionCollection}/`;
    this.seen = new Set(); // `${did}:${seq}` keys
    this.sockets = []; // { ws, key, dids: Set }
    this.stopped = false;
    this.reconnectDelays = new Map(); // socket key → ms
    this.cursorBySocket = new Map(); // socket key → last firehose seq seen
    this.pdsByDid = new Map(); // did → pdsUri (always populated for backfill)
    this._lastEventAt = 0; // when the last NEW action was dispatched
    this._lastSweepAt = 0; // when the last backfill sweep started
    this._sweeping = false;
    this._pollTimer = null;
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

    // Open the live subscription FIRST; backfill only once it's connected.
    // Anything published before the socket went live is on the PDS for the
    // backfill to find, anything after flows down the wire, and the overlap
    // dedups by (did, seq) — no gap.
    const opens = [];
    const filteredFirehose = import.meta.env.VITE_FIREHOSE_URL;
    if (filteredFirehose) {
      // Single connection to the filtered firehose service.
      const dids = new Set(this.peerDids);
      opens.push(
        this._openSocket({
          key: "filtered",
          wsBase: originToWs(filteredFirehose),
          dids,
          wantedDids: this.peerDids,
        }),
      );
    } else {
      // Fallback: one socket per unique PDS.
      const byPds = new Map(); // pdsUri → did[]
      for (const [did, pds] of this.pdsByDid) {
        if (!byPds.has(pds)) byPds.set(pds, []);
        byPds.get(pds).push(did);
      }
      for (const [pds, dids] of byPds) {
        opens.push(
          this._openSocket({
            key: pds,
            wsBase: originToWs(pds),
            dids: new Set(dids),
          }),
        );
      }
    }
    await Promise.race([
      Promise.all(opens),
      new Promise((resolve) => setTimeout(resolve, SOCKET_OPEN_TIMEOUT_MS)),
    ]);

    await this.backfillAll();

    // Quiet-period safety net: if no action has arrived for a while, sweep
    // the PDSes for anything a lost frame would otherwise have buried.
    this._pollTimer = setInterval(() => {
      const idleSince = Math.max(this._lastEventAt, this._lastSweepAt);
      if (Date.now() - idleSince >= QUIET_RESWEEP_MS) this.backfillAll();
    }, RESWEEP_POLL_MS);
  }

  /** Sweep every peer's PDS for action records. Safe to call repeatedly. */
  async backfillAll() {
    if (this._sweeping || this.stopped) return;
    this._sweeping = true;
    this._lastSweepAt = Date.now();
    try {
      await Promise.all(this.peerDids.map((did) => this._backfill(did)));
    } finally {
      this._sweeping = false;
    }
  }

  stop() {
    this.stopped = true;
    if (this._pollTimer) {
      clearInterval(this._pollTimer);
      this._pollTimer = null;
    }
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
      // Page through the WHOLE collection — a long game (or a spectator
      // replaying one from scratch) has far more than one page of actions.
      // reverse=true gives ascending rkey order, which matches seq order now
      // that action rkeys are zero-padded.
      let cursor;
      do {
        const url =
          `${pds}/xrpc/com.atproto.repo.listRecords` +
          `?repo=${encodeURIComponent(did)}` +
          `&collection=${encodeURIComponent(this.actionCollection)}` +
          `&limit=100&reverse=true` +
          (cursor ? `&cursor=${encodeURIComponent(cursor)}` : "");
        const res = await fetch(url);
        if (!res.ok) return;
        const data = await res.json();
        const records = data.records || [];
        for (const r of records) {
          if (r.value?.table?.uri !== this.tableUri) continue;
          const seq = r.value.seq;
          this._dispatch(did, seq, () => this._actionFromJsonRecord(r.value));
        }
        cursor = records.length > 0 ? data.cursor : undefined;
      } while (cursor);
    } catch (e) {
      console.warn(`[firehose] backfill ${did} failed:`, e?.message || e);
    }
  }

  /**
   * Open one subscribeRepos socket. Resolves once the connection is open (or
   * has failed and entered the reconnect loop) — start() backfills only after
   * this, so nothing can slip between snapshot and subscription.
   */
  _openSocket({ key, wsBase, dids, wantedDids }) {
    if (this.stopped) return Promise.resolve();
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

    let resolveSettled;
    const settled = new Promise((resolve) => {
      resolveSettled = resolve;
    });

    ws.addEventListener("open", () => {
      this.reconnectDelays.set(key, 0);
      resolveSettled();
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
        const hasActionOp = (frame.ops || []).some((op) =>
          op.path?.startsWith(this.actionPathPrefix),
        );
        if (!hasActionOp) return;

        for await (const record of extractActionRecords(
          new Uint8Array(frame.blocks),
          this.actionCollection,
        )) {
          if (record.table?.uri !== this.tableUri) continue;
          const seq = record.seq;
          this._dispatch(frame.repo, seq, () => this._cborFromRecord(record));
        }
      } catch (e) {
        console.warn("[firehose] frame error:", e?.message || e);
      }
    });

    ws.addEventListener("close", () => {
      resolveSettled();
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

    return settled;
  }

  _dispatch(did, seq, makeCbor) {
    if (this.stopped) return;
    const dedupKey = `${did}:${seq}`;
    if (this.seen.has(dedupKey)) return;
    this.seen.add(dedupKey);
    this._lastEventAt = Date.now();
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
