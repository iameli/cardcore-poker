/**
 * AT Protocol record publisher for the card-game protocols.
 *
 * Players publish their actions as `re.cardco.{poker,blackjack}.action`
 * records to their own PDS. Reads happen via `lib/firehose.js`'s
 * subscribeRepos subscription.
 */
import * as dagCbor from "@ipld/dag-cbor";
import { buildActionRecord, buildTableRecord, LEXICONS, TABLE_COLLECTIONS, } from "./atproto-publisher.js";
import { pdsForDid } from "./atproto.js";

/**
 * Fetch a table record by AT URI from its author's PDS (getRecord is public,
 * no auth needed). Returns { record, cid }.
 */
export async function fetchTableRecord(uri, ownPdsUri) {
  const m = uri.match(/^at:\/\/([^/]+)\/([^/]+)\/(.+)$/);
  if (!m) throw new Error(`Bad table URI: ${uri}`);
  const [, repo, collection, rkey] = m;
  if (!TABLE_COLLECTIONS.includes(collection)) {
    throw new Error(`URI is not a known game table: ${collection}`);
  }
  const pds = await pdsForDid(repo, ownPdsUri);
  const url =
    `${pds}/xrpc/com.atproto.repo.getRecord` +
    `?repo=${encodeURIComponent(repo)}` +
    `&collection=${encodeURIComponent(collection)}` +
    `&rkey=${encodeURIComponent(rkey)}`;
  const res = await fetch(url);
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`getRecord(${repo}) ${res.status}: ${body.slice(0, 200)}`);
  }
  const data = await res.json();
  return { record: data.value, cid: data.cid };
}

/**
 * Action rkeys are `<tableTid>-<seq>` with the seq zero-padded to 9 digits so
 * lexicographic rkey order matches numeric seq order (unpadded, "-10" sorted
 * before "-2"). Caps a game at 1,000,000,000 actions, which is plenty.
 */
function rkeyForSeq(tableTid, seq) {
  if (seq >= 1_000_000_000) throw new Error(`action seq ${seq} exceeds rkey padding`);
  return `${tableTid}-${String(seq).padStart(9, "0")}`;
}

/**
 * Fired on `window` when the PDS rejects our credentials (expired/invalid
 * token). App.svelte listens, bounces the user through sign-in, and returns
 * them to the page they were on.
 */
export const AUTH_EXPIRED_EVENT = "cardcore:auth-expired";

function notifyAuthExpired() {
  try {
    window.dispatchEvent(new CustomEvent(AUTH_EXPIRED_EVENT));
  } catch {
    // not in a browser (tests/tools) — caller still gets the thrown error
  }
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
 * Publishes records to the player's PDS via the @atcute Client we already
 * built during signin. Collections default to poker; pass a game's
 * `tableCollection`/`actionCollection` to publish for another game.
 */
export class Publisher {
  constructor({
    client,
    did,
    tableCollection = LEXICONS.TABLE,
    actionCollection = LEXICONS.ACTION,
  }) {
    this.client = client;
    this.did = did;
    this.tableCollection = tableCollection;
    this.actionCollection = actionCollection;
  }

  async createTable({ players, startingChips, smallBlind, minBet }) {
    const record = buildTableRecord({
      collection: this.tableCollection,
      players,
      startingChips,
      smallBlind,
      minBet,
    });
    return this._createRecord(this.tableCollection, record);
  }

  async publishTableWithRkey(
    rkey,
    { players, startingChips, smallBlind, minBet, startedAt, updatedAt },
  ) {
    const record = buildTableRecord({
      collection: this.tableCollection,
      players,
      startingChips,
      smallBlind,
      minBet,
      startedAt,
      updatedAt,
    });
    return this._createWithRkey(this.tableCollection, rkey, record);
  }

  /**
   * Publish a single game action. Used for actions the local agent emits.
   * `actionCbor` is the raw CBOR the WASM agent produced; we decode it to
   * reconstitute the inner action object as the lexicon expects.
   */
  async publishAction({ tableRef, prevRef, seq, tableTid, actionCbor }) {
    // Decode the WASM-emitted CBOR, then dehydrate Uint8Array fields to the
    // `{ $bytes: base64 }` JSON wire format the lexicon expects. Without this
    // step, @atcute's JSON.stringify turns Uint8Arrays into numeric-keyed
    // objects, which the Rust lexicon parser rejects on read.
    const innerAction = dehydrateBytes(dagCbor.decode(actionCbor));
    const record = buildActionRecord({
      collection: this.actionCollection,
      tableRef,
      prevRef,
      seq,
      action: innerAction,
    });
    return this._createWithRkey(this.actionCollection, rkeyForSeq(tableTid, seq), record);
  }

  async _createRecord(collection, record) {
    const res = await this.client.post("com.atproto.repo.createRecord", {
      input: { repo: this.did, collection, record },
    });
    if (!res.ok) {
      if (res.status === 401) notifyAuthExpired();
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
      if (res.status === 401) notifyAuthExpired();
      throw new Error(
        `putRecord(${collection}/${rkey}) failed: ${res.status} ${JSON.stringify(res.data)}`,
      );
    }
    return { uri: res.data.uri, cid: res.data.cid };
  }
}
