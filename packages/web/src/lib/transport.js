/**
 * AT Protocol record publisher for the poker protocol.
 *
 * Players publish their actions as `re.cardco.poker.action` records to their
 * own PDS. Reads happen via `lib/firehose.js`'s subscribeRepos subscription.
 */
import * as dagCbor from "@ipld/dag-cbor";
import { buildActionRecord, buildTableRecord, LEXICONS } from "./atproto-publisher.js";
import { pdsForDid } from "./atproto.js";

/**
 * Fetch a table record by AT URI from its author's PDS (getRecord is public,
 * no auth needed). Returns { record, cid }.
 */
export async function fetchTableRecord(uri, ownPdsUri) {
  const m = uri.match(/^at:\/\/([^/]+)\/([^/]+)\/(.+)$/);
  if (!m) throw new Error(`Bad table URI: ${uri}`);
  const [, repo, collection, rkey] = m;
  if (collection !== LEXICONS.TABLE) {
    throw new Error(`URI is not a poker table: ${collection}`);
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
 * Deterministic JSON with sorted object keys, for comparing records that may
 * have passed through different serializers.
 */
function stableStringify(v) {
  if (v === null || typeof v !== "object") return JSON.stringify(v);
  if (Array.isArray(v)) return "[" + v.map(stableStringify).join(",") + "]";
  return (
    "{" +
    Object.keys(v)
      .sort()
      .map((k) => JSON.stringify(k) + ":" + stableStringify(v[k]))
      .join(",") +
    "}"
  );
}

/**
 * Canonicalize a record for comparison: `$bytes` values lose their base64
 * padding (we btoa-pad on write; the PDS serves them unpadded per the atproto
 * data model), and nested objects/arrays are walked.
 */
function canonicalize(value) {
  if (value == null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map(canonicalize);
  const out = {};
  for (const [k, v] of Object.entries(value)) {
    out[k] = k === "$bytes" && typeof v === "string" ? v.replace(/=+$/, "") : canonicalize(v);
  }
  return out;
}

/**
 * Whether two action records carry the same game content. `createdAt` is
 * excluded: a resumed session re-derives the same action at a later wall
 * clock, and that must still count as "the record I already published".
 */
function sameActionContent(a, b) {
  const strip = ({ createdAt, ...rest }) => rest;
  return stableStringify(canonicalize(strip(a))) === stableStringify(canonicalize(strip(b)));
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
 * Compute a record's DASL CID string: CIDv1, dag-cbor codec, sha-256,
 * base32-lower multibase — the same CID the PDS derives when it commits the
 * record. `record` must be in data-model form (real Uint8Arrays, not
 * `{ $bytes }` wrappers).
 */
export async function computeRecordCid(record) {
  const body = dagCbor.encode(record);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", body));
  // <cidv1> <dag-cbor> <sha2-256> <32 bytes> || digest
  const bytes = new Uint8Array(4 + digest.length);
  bytes.set([0x01, 0x71, 0x12, 0x20]);
  bytes.set(digest, 4);
  return "b" + base32Lower(bytes);
}

/** RFC 4648 base32, lowercase, no padding (the multibase 'b' alphabet). */
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
 * Publishes records to the player's PDS via the @atcute Client we already
 * built during signin.
 */
export class Publisher {
  constructor({ client, did }) {
    this.client = client;
    this.did = did;
    // rkey → { uri, cid, value } of action records this repo has ALREADY
    // published for the current table. Loaded once at session start so a
    // resumed session can verify its re-derived history against the repo
    // locally instead of re-publishing it. See loadPublishedActions().
    this._published = new Map();
  }

  /**
   * Snapshot this repo's existing action records for a table. Call once
   * before the session starts publishing: occupied slots are then verified
   * by CID comparison with zero HTTP requests, and only genuinely new
   * actions hit the wire. Without the snapshot everything still works —
   * occupied slots just fall through to the slower create-then-probe path.
   */
  async loadPublishedActions(tableTid) {
    this._published = new Map();
    const prefix = `${tableTid}-`;
    let cursor;
    do {
      const params = { repo: this.did, collection: LEXICONS.ACTION, limit: 100 };
      if (cursor) params.cursor = cursor;
      const res = await this.client.get("com.atproto.repo.listRecords", { params });
      if (!res.ok) {
        throw new Error(`listRecords(${LEXICONS.ACTION}) failed: ${res.status}`);
      }
      for (const rec of res.data.records || []) {
        const rkey = rec.uri.split("/").pop();
        if (rkey.startsWith(prefix)) {
          this._published.set(rkey, { uri: rec.uri, cid: rec.cid, value: rec.value });
        }
      }
      cursor = res.data.records?.length ? res.data.cursor : undefined;
    } while (cursor);
    return this._published.size;
  }

  async createTable({ players, startingChips, smallBlind }) {
    const record = buildTableRecord({ players, startingChips, smallBlind });
    return this._createRecord(LEXICONS.TABLE, record);
  }

  async publishTableWithRkey(rkey, { players, startingChips, smallBlind, startedAt, updatedAt }) {
    const record = buildTableRecord({ players, startingChips, smallBlind, startedAt, updatedAt });
    // Tables are the one legitimately mutable record: the open-room flow
    // updates the roster/startedAt in place so the table URL stays stable.
    // Everything else (actions) is create-only via _createWithRkey.
    return this._putWithRkey(LEXICONS.TABLE, rkey, record);
  }

  /**
   * Publish a single poker action. Used for actions the local agent emits.
   * `actionCbor` is the raw CBOR the WasmAgent produced; we decode it to
   * reconstitute the inner action object as the lexicon expects.
   */
  async publishAction({ tableRef, prevRef, seq, tableTid, actionCbor }) {
    const rkey = rkeyForSeq(tableTid, seq);
    const rawAction = dagCbor.decode(actionCbor);

    // Slot already published by a previous session of ours (reload replay
    // re-derives deterministic history)? Verify locally by CID and skip the
    // wire entirely. createdAt is adopted from the published record — it's
    // the one field a re-derivation legitimately can't reproduce.
    const known = this._published.get(rkey);
    if (known) {
      const proposed = buildActionRecord({ tableRef, prevRef, seq, action: rawAction });
      proposed.createdAt = known.value.createdAt;
      const cid = await computeRecordCid(proposed);
      if (cid === known.cid) {
        return { uri: known.uri, cid: known.cid };
      }
      // A CID mismatch could still be an encoding subtlety rather than real
      // divergence — fall back to a semantic compare before condemning it.
      if (sameActionContent(known.value, dehydrateBytes(proposed))) {
        console.warn(
          `[publisher] ${rkey}: content matches but computed CID ${cid} != repo CID ${known.cid}`,
        );
        return { uri: known.uri, cid: known.cid };
      }
      throw new Error(
        `refusing to overwrite ${LEXICONS.ACTION}/${rkey}: the record already on ` +
          `the PDS differs from the locally replayed action — local state has ` +
          `diverged from published history`,
      );
    }

    // Decode the WASM-emitted CBOR, then dehydrate Uint8Array fields to the
    // `{ $bytes: base64 }` JSON wire format the lexicon expects. Without this
    // step, @atcute's JSON.stringify turns Uint8Arrays into numeric-keyed
    // objects, which the Rust lexicon parser rejects on read.
    const record = buildActionRecord({
      tableRef,
      prevRef,
      seq,
      action: dehydrateBytes(rawAction),
    });
    return this._createWithRkey(LEXICONS.ACTION, rkey, record);
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

  /**
   * Create-only publish at a fixed rkey — game actions are immutable history.
   * putRecord would silently overwrite whatever is already at the rkey;
   * that's how the original hand-1 records of the first real game were lost
   * when a resumed client re-published old slots. createRecord fails if the
   * rkey exists, so:
   *   - slot free            → normal create
   *   - slot has SAME content (modulo createdAt) → idempotent success; a
   *     reload legitimately re-derives and re-publishes its own history
   *   - slot has DIFFERENT content → hard error; the local replay diverged
   *     from published history and must not clobber the record peers saw
   */
  async _createWithRkey(collection, rkey, record) {
    const res = await this.client.post("com.atproto.repo.createRecord", {
      input: { repo: this.did, collection, rkey, record },
    });
    if (res.ok) return { uri: res.data.uri, cid: res.data.cid };
    if (res.status === 401) {
      notifyAuthExpired();
      throw new Error(
        `createRecord(${collection}/${rkey}) failed: ${res.status} ${JSON.stringify(res.data)}`,
      );
    }

    // The create may have failed because the slot is occupied — check.
    const existing = await this._getOwnRecord(collection, rkey);
    if (existing) {
      if (sameActionContent(existing.value, record)) {
        return { uri: existing.uri, cid: existing.cid };
      }
      throw new Error(
        `refusing to overwrite ${collection}/${rkey}: the record already on the ` +
          `PDS differs from the locally replayed action — local state has ` +
          `diverged from published history`,
      );
    }
    throw new Error(
      `createRecord(${collection}/${rkey}) failed: ${res.status} ${JSON.stringify(res.data)}`,
    );
  }

  /**
   * A record from the loadPublishedActions() snapshot, or null. Lets the
   * caller inspect what this repo already published at a seq slot — e.g. to
   * verify a locally stored seed can actually reproduce the game's history
   * before letting an agent participate.
   */
  publishedRecord(tableTid, seq) {
    return this._published.get(rkeyForSeq(tableTid, seq)) || null;
  }

  async _getOwnRecord(collection, rkey) {
    const res = await this.client.get("com.atproto.repo.getRecord", {
      params: { repo: this.did, collection, rkey },
    });
    return res.ok ? res.data : null;
  }

  async _putWithRkey(collection, rkey, record) {
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
