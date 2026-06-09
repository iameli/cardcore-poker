/**
 * Join-request discovery for the open-room lobby.
 *
 * When a host opens a room, they don't yet know who will ask to join, so they
 * can't filter a firehose by `wantedDids`. Instead the host watches for
 * `re.cardco.poker.table` records published at *their* table's rkey by *other*
 * repos — each such record is a join request listing `[host, …, joiner]`.
 *
 * Two modes mirror lib/firehose.js:
 *
 *  1. **Jetstream** (prod) — when `VITE_JETSTREAM_URL` is set, open one socket
 *     to `${url}/subscribe?wantedCollections=re.cardco.poker.table`. Jetstream
 *     filters by collection across the whole network, which is exactly what
 *     discovery needs (the joiner's DID is unknown up front). Messages are
 *     plain JSON.
 *
 *  2. **Per-PDS subscribeRepos** (dev fallback) — when no Jetstream is
 *     configured, subscribe to the host's own PDS `com.atproto.sync.subscribeRepos`
 *     with no `wantedDids` and filter client-side. The local dev PDS only hosts
 *     our demo accounts, so this is cheap.
 *
 * IMPORTANT: this is DISCOVERY ONLY. Jetstream is a re-serialized, unsigned
 * convenience stream — fine for "who wants to play" but NOT trustworthy for
 * gameplay. Once the host starts the hand, players verify the host's table
 * record directly and exchange moves over the real (CAR-backed, signed)
 * firehose via wantedDids in GameRoom. The watcher is torn down before then.
 */
import { decodeMultiple } from "cbor-x";
import { CarReader } from "@ipld/car";
import * as dagCbor from "@ipld/dag-cbor";
import { LEXICONS } from "./atproto-publisher.js";

const TABLE_PATH_PREFIX = `${LEXICONS.TABLE}/`;

function decodeFrame(bytes) {
  const values = [];
  decodeMultiple(bytes, (v) => values.push(v));
  if (values.length < 2) return null;
  const [header, body] = values;
  if (header?.op !== 1) return null;
  return { t: header.t, ...body };
}

function originToWs(uri) {
  if (uri.startsWith("ws://") || uri.startsWith("wss://")) return uri;
  return uri.replace(/^http/, "ws");
}

async function* extractTableRecords(carBytes) {
  const reader = await CarReader.fromBytes(carBytes);
  for await (const block of reader.blocks()) {
    let record;
    try {
      record = dagCbor.decode(block.bytes);
    } catch {
      continue;
    }
    if (record?.$type === LEXICONS.TABLE) yield record;
  }
}

export class JoinRequestWatcher {
  /**
   * @param {object} opts
   * @param {string} opts.hostDid - the room host's DID (us)
   * @param {string} opts.tableRkey - rkey of the host's table record
   * @param {string} opts.ownPdsUri - host's PDS endpoint (dev fallback target)
   * @param {(req: {joinerDid: string, players: string[], createdAt?: string}) => void} opts.onRequest
   */
  constructor({ hostDid, tableRkey, ownPdsUri, onRequest }) {
    this.hostDid = hostDid;
    this.tableRkey = tableRkey;
    this.ownPdsUri = ownPdsUri;
    this.onRequest = onRequest;
    this.seen = new Set(); // joiner DIDs already surfaced
    this.ws = null;
    this.stopped = false;
    this.reconnectDelay = 0;
  }

  start() {
    const jetstream = import.meta.env.VITE_JETSTREAM_URL;
    if (jetstream) this._connectJetstream(jetstream);
    else this._connectSubscribeRepos();
  }

  stop() {
    this.stopped = true;
    try {
      this.ws?.close();
    } catch {}
    this.ws = null;
  }

  _emit(joinerDid, record) {
    if (!joinerDid || joinerDid === this.hostDid) return; // ignore our own commits
    if (!Array.isArray(record?.players)) return;
    if (!record.players.includes(this.hostDid)) return; // not for our table
    if (this.seen.has(joinerDid)) return;
    this.seen.add(joinerDid);
    try {
      this.onRequest({
        joinerDid,
        players: record.players,
        createdAt: record.createdAt,
      });
    } catch (e) {
      console.warn("[discovery] onRequest threw:", e?.message || e);
    }
  }

  _reconnect(connect) {
    if (this.stopped) return;
    const prev = this.reconnectDelay;
    const next = prev === 0 ? 1000 : Math.min(prev * 2, 30_000);
    this.reconnectDelay = next;
    setTimeout(() => {
      if (!this.stopped) connect();
    }, next);
  }

  // ─── Jetstream (prod) ────────────────────────────────────────────
  _connectJetstream(base) {
    if (this.stopped) return;
    const url =
      `${originToWs(base)}/subscribe` + `?wantedCollections=${encodeURIComponent(LEXICONS.TABLE)}`;
    const ws = new WebSocket(url);
    this.ws = ws;

    ws.addEventListener("open", () => {
      this.reconnectDelay = 0;
    });

    ws.addEventListener("message", (ev) => {
      try {
        const msg = JSON.parse(ev.data);
        if (msg.kind !== "commit") return;
        const c = msg.commit;
        if (!c || c.collection !== LEXICONS.TABLE) return;
        if (c.rkey !== this.tableRkey) return;
        if (c.operation !== "create" && c.operation !== "update") return;
        this._emit(msg.did, c.record);
      } catch (e) {
        console.warn("[discovery] jetstream message error:", e?.message || e);
      }
    });

    ws.addEventListener("close", () => this._reconnect(() => this._connectJetstream(base)));
    ws.addEventListener("error", () => {
      /* close handler retries */
    });
  }

  // ─── Per-PDS subscribeRepos (dev fallback) ───────────────────────
  _connectSubscribeRepos() {
    if (this.stopped) return;
    const url = `${originToWs(this.ownPdsUri)}/xrpc/com.atproto.sync.subscribeRepos`;
    const ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";
    this.ws = ws;

    ws.addEventListener("open", () => {
      this.reconnectDelay = 0;
    });

    ws.addEventListener("message", async (ev) => {
      try {
        const frame = decodeFrame(new Uint8Array(ev.data));
        if (!frame || frame.t !== "#commit") return;
        if (frame.repo === this.hostDid) return;
        if (!frame.blocks) return;
        const touchesTable = (frame.ops || []).some(
          (op) => op.path === `${TABLE_PATH_PREFIX}${this.tableRkey}`,
        );
        if (!touchesTable) return;
        for await (const record of extractTableRecords(new Uint8Array(frame.blocks))) {
          this._emit(frame.repo, record);
        }
      } catch (e) {
        console.warn("[discovery] frame error:", e?.message || e);
      }
    });

    ws.addEventListener("close", () => this._reconnect(() => this._connectSubscribeRepos()));
    ws.addEventListener("error", () => {
      /* close handler retries */
    });
  }
}
