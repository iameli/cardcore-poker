/**
 * AT Protocol record publisher for the poker protocol.
 *
 * Players publish their actions as `re.cardco.poker.action` records to their
 * own PDS. Reads happen via `lib/firehose.js`'s subscribeRepos subscription.
 */
import * as dagCbor from "@ipld/dag-cbor";
import { buildActionRecord, buildTableRecord, LEXICONS } from "./atproto-publisher.js";

function rkeyForSeq(tableTid, seq) {
  return `${tableTid}-${seq}`;
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
