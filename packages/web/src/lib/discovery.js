/**
 * Join-request discovery for the open-room lobby.
 *
 * When a host opens a room they don't know who will ask to join, so they can't
 * filter a firehose by `wantedDids`. Instead the host subscribes to the FULL
 * unfiltered firehose (`com.atproto.sync.subscribeRepos` with no `wantedDids`)
 * and watches for `re.cardco.poker.table` records published at *their* table's
 * rkey by *other* repos — each such record is a join request listing
 * `[host, …, joiner]`.
 *
 * Endpoint:
 *  - **prod**: `VITE_RELAY_URL` (the network relay, e.g. wss://bsky.network),
 *    which streams the whole network so joiners on any PDS are visible.
 *  - **dev**:  the host's own PDS (`ownPdsUri`) — the local dev PDS hosts all
 *    our demo accounts, so its firehose already includes every joiner.
 *
 * This runs ONLY in the lobby. The watcher is stopped the moment the host
 * starts the hand (RoomLobby unmounts) — we don't carry the full firehose into
 * gameplay, which subscribes to the filtered (wantedDids) firehose in GameRoom.
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
    // Prod: the network relay (whole-network firehose). Dev: our own PDS, which
    // hosts every demo account. Either way it's the unfiltered subscribeRepos
    // stream — the host doesn't know joiner DIDs up front, so it can't filter.
    const base = import.meta.env.VITE_RELAY_URL || this.ownPdsUri;
    this._connect(base);
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

  _connect(base) {
    if (this.stopped) return;
    const url = `${originToWs(base)}/xrpc/com.atproto.sync.subscribeRepos`;
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

    ws.addEventListener("close", () => this._reconnect(() => this._connect(base)));
    ws.addEventListener("error", () => {
      /* close handler retries */
    });
  }
}
