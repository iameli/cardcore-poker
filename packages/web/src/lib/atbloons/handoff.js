/**
 * atbloons wallet handoff — Cardcore client contract.
 *
 * This module is the Cardcore side of the protocol-v3 atbloons wallet handoff.
 * It builds the public intent, opens the confidential wallet, and reads the
 * public receipt fragment on return. It never sees an OAuth token, a refresh
 * token, or a DPoP key. It sends only public game references and reads only a
 * public receipt.
 *
 * The wire contract is the atbloons public API in
 * `docs/cardcore-wallet-handoff.md`. Every value here matches the strict
 * atbloons DTO in `CardCoreWalletHandoffDtos.kt`. The wallet re-verifies all
 * repository evidence through its local node before it spends. atbloons is a
 * resettable testnet, not a blockchain or mainnet.
 *
 * The module has no dependencies. It runs in a browser and under `node --test`.
 */

// ─── Fixed wire constants ──────────────────────────────────────────

export const HANDOFF_VERSION = "cardcore-poker-contract-v1";
export const PROTOCOL_VERSION = "v3";

export const COMMAND_KIND = Object.freeze({
  PROPOSE: "PROPOSE",
  FUND: "FUND",
  ACTIVATE: "ACTIVATE",
  WITHDRAW: "WITHDRAW",
  SETTLE: "SETTLE",
});

export const RECEIPT_STATUS = Object.freeze({
  COMPLETE: "complete",
  RETRYABLE: "retryable",
  FAILED: "failed",
});

export const ERROR_CODE = Object.freeze({
  INVALID_REQUEST: "invalid_request",
  WRONG_NETWORK: "wrong_network",
  WRONG_ACCOUNT: "wrong_account",
  WRONG_ORIGIN: "wrong_origin",
  GRANT_REQUIRED: "grant_required",
  STALE_CONTINUATION: "stale_continuation",
  ACCOUNT_NOT_READY: "account_not_ready",
  CONTRACT_NOT_READY: "contract_not_ready",
  CONTRACT_TERMINAL: "contract_terminal",
  CARDCORE_INVALID: "cardcore_invalid",
  REAUTHENTICATION_REQUIRED: "reauthentication_required",
  PUBLICATION_PENDING: "publication_pending",
  PUBLICATION_FAILED: "publication_failed",
});

/** Record collections a receipt may report as published. */
export const CONTRACT_COLLECTION = Object.freeze({
  CONTRACT: "tech.lenooby09.atbloons.contract",
  FUNDING: "tech.lenooby09.atbloons.contractFunding",
  WITHDRAWAL: "tech.lenooby09.atbloons.contractWithdrawal",
  ACTIVATION: "tech.lenooby09.atbloons.contractActivation",
  SETTLEMENT: "tech.lenooby09.atbloons.contractSettlement",
});

/** Cardcore record collections that the intent references. */
export const CARDCORE_COLLECTION = Object.freeze({
  TABLE: "re.cardco.poker.table",
  ACTION: "re.cardco.poker.action",
});

const PUBLISHED_COLLECTIONS = new Set(Object.values(CONTRACT_COLLECTION));
const RECEIPT_STATUSES = new Set(Object.values(RECEIPT_STATUS));
const ERROR_CODES = new Set(Object.values(ERROR_CODE));
const TERMINAL_COMMANDS = new Set([COMMAND_KIND.WITHDRAW, COMMAND_KIND.SETTLE]);
const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "::1"]);

const MAX_ENCODED_INTENT_CHARS = 8192;
const MAX_DECODED_INTENT_BYTES = 4096;
const MAX_RETURN_URL_LENGTH = 2048;
const CONTINUATION_LENGTH = 43;
const STATE_BYTE_MIN = 16;
const STATE_BYTE_MAX = 64;

const NETWORK_ID_PATTERN = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/;
const GENESIS_HASH_PATTERN = /^[0-9a-f]{64}$/;
const AMOUNT_PATTERN = /^(0|[1-9][0-9]*)$/;
const BASE64_URL_PATTERN = /^[A-Za-z0-9_-]+$/;
const AT_URI_PREFIX = "at://";

/** A caller mistake in this module. Not a wallet failure. */
export class HandoffError extends Error {
  constructor(message) {
    super(message);
    this.name = "HandoffError";
  }
}

// ─── base64url over UTF-8 JSON (portable, no dependency) ────────────

const B64_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const B64_LOOKUP = (() => {
  const table = new Int16Array(128).fill(-1);
  for (let i = 0; i < B64_ALPHABET.length; i++) table[B64_ALPHABET.charCodeAt(i)] = i;
  return table;
})();

/** Encode raw bytes as unpadded URL-safe base64. */
export function bytesToBase64Url(bytes) {
  let out = "";
  let i = 0;
  for (; i + 3 <= bytes.length; i += 3) {
    const n = (bytes[i] << 16) | (bytes[i + 1] << 8) | bytes[i + 2];
    out += B64_ALPHABET[(n >> 18) & 63];
    out += B64_ALPHABET[(n >> 12) & 63];
    out += B64_ALPHABET[(n >> 6) & 63];
    out += B64_ALPHABET[n & 63];
  }
  const rem = bytes.length - i;
  if (rem === 1) {
    const n = bytes[i] << 16;
    out += B64_ALPHABET[(n >> 18) & 63];
    out += B64_ALPHABET[(n >> 12) & 63];
  } else if (rem === 2) {
    const n = (bytes[i] << 16) | (bytes[i + 1] << 8);
    out += B64_ALPHABET[(n >> 18) & 63];
    out += B64_ALPHABET[(n >> 12) & 63];
    out += B64_ALPHABET[(n >> 6) & 63];
  }
  return out;
}

/** Decode unpadded URL-safe base64 into raw bytes. Rejects padded input. */
export function base64UrlToBytes(value) {
  if (typeof value !== "string" || value.length === 0 || !BASE64_URL_PATTERN.test(value)) {
    throw new HandoffError("Value is not unpadded base64url");
  }
  if (value.length % 4 === 1) throw new HandoffError("Value is not valid base64url");
  const out = new Uint8Array(Math.floor((value.length * 3) / 4));
  let outIndex = 0;
  let i = 0;
  for (; i + 4 <= value.length; i += 4) {
    const n =
      (B64_LOOKUP[value.charCodeAt(i)] << 18) |
      (B64_LOOKUP[value.charCodeAt(i + 1)] << 12) |
      (B64_LOOKUP[value.charCodeAt(i + 2)] << 6) |
      B64_LOOKUP[value.charCodeAt(i + 3)];
    out[outIndex++] = (n >> 16) & 255;
    out[outIndex++] = (n >> 8) & 255;
    out[outIndex++] = n & 255;
  }
  const rem = value.length - i;
  if (rem === 2) {
    const n = (B64_LOOKUP[value.charCodeAt(i)] << 18) | (B64_LOOKUP[value.charCodeAt(i + 1)] << 12);
    out[outIndex++] = (n >> 16) & 255;
  } else if (rem === 3) {
    const n =
      (B64_LOOKUP[value.charCodeAt(i)] << 18) |
      (B64_LOOKUP[value.charCodeAt(i + 1)] << 12) |
      (B64_LOOKUP[value.charCodeAt(i + 2)] << 6);
    out[outIndex++] = (n >> 16) & 255;
    out[outIndex++] = (n >> 8) & 255;
  }
  return out.subarray(0, outIndex);
}

const TEXT_ENCODER = new TextEncoder();
const TEXT_DECODER = new TextDecoder("utf-8", { fatal: true });

function stringToBase64Url(text) {
  return bytesToBase64Url(TEXT_ENCODER.encode(text));
}

function base64UrlToString(value) {
  return TEXT_DECODER.decode(base64UrlToBytes(value));
}

// ─── random state (128 or more random bits) ────────────────────────

function randomBytes(length) {
  const bytes = new Uint8Array(length);
  const crypto = globalThis.crypto;
  if (!crypto || typeof crypto.getRandomValues !== "function") {
    throw new HandoffError("A secure random source is required for the handoff state");
  }
  crypto.getRandomValues(bytes);
  return bytes;
}

/** A fresh 128-bit opaque correlation value as unpadded base64url. */
export function newHandoffState() {
  return bytesToBase64Url(randomBytes(16));
}

// ─── validation that mirrors the atbloons DTO ──────────────────────

function requireString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    throw new HandoffError(`${label} must be a non-empty string`);
  }
  return value;
}

/** Reject a field that belongs to another command kind. */
function requireAbsent(spec, fields, kind) {
  for (const field of fields) {
    if (spec[field] != null) throw new HandoffError(`${kind} cannot carry ${field}`);
  }
}

function requireStrongRef(ref, collection, label) {
  if (!ref || typeof ref !== "object")
    throw new HandoffError(`${label} must be a strong reference`);
  const uri = requireString(ref.uri, `${label} uri`);
  const cid = requireString(ref.cid, `${label} cid`);
  if (cid.length > 256) throw new HandoffError(`${label} cid is too long`);
  if (!uri.startsWith(AT_URI_PREFIX)) throw new HandoffError(`${label} uri must be an AT URI`);
  const collectionOfUri = uri.slice(AT_URI_PREFIX.length).split("/")[1];
  if (collectionOfUri !== collection) {
    throw new HandoffError(`${label} must reference the ${collection} collection`);
  }
  return { uri, cid };
}

function requireScope(scope) {
  if (!scope || typeof scope !== "object") throw new HandoffError("scope must be a network tuple");
  const networkId = requireString(scope.networkId, "scope networkId");
  const protocolVersion = requireString(scope.protocolVersion, "scope protocolVersion");
  const genesisHash = requireString(scope.genesisHash, "scope genesisHash");
  if (!NETWORK_ID_PATTERN.test(networkId)) throw new HandoffError("scope networkId is invalid");
  if (protocolVersion !== PROTOCOL_VERSION) {
    throw new HandoffError("The atbloons wallet handoff requires protocol version v3");
  }
  if (!GENESIS_HASH_PATTERN.test(genesisHash))
    throw new HandoffError("scope genesisHash is invalid");
  return { networkId, protocolVersion, genesisHash };
}

function requireReturnUrl(value) {
  requireString(value, "returnUrl");
  if (value.length > MAX_RETURN_URL_LENGTH) throw new HandoffError("returnUrl is too long");
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new HandoffError("returnUrl is not a valid absolute URL");
  }
  if (url.username || url.password)
    throw new HandoffError("returnUrl cannot include user information");
  if (url.hash) throw new HandoffError("returnUrl cannot include a fragment");
  const host = url.hostname.replace(/^\[/, "").replace(/\]$/, "").toLowerCase();
  if (!host) throw new HandoffError("returnUrl must include a host");
  const scheme = url.protocol.replace(/:$/, "").toLowerCase();
  if (scheme !== "https" && !(scheme === "http" && LOOPBACK_HOSTS.has(host))) {
    throw new HandoffError("returnUrl must use HTTPS or exact loopback HTTP");
  }
  return value;
}

function requireState(value) {
  requireString(value, "state");
  if (!BASE64_URL_PATTERN.test(value)) throw new HandoffError("state must be unpadded base64url");
  const bytes = base64UrlToBytes(value);
  if (bytes.length < STATE_BYTE_MIN || bytes.length > STATE_BYTE_MAX) {
    throw new HandoffError("state must carry 16 to 64 random bytes");
  }
  return value;
}

function requireSoulsPerChip(value) {
  requireString(value, "soulsPerChip");
  if (!AMOUNT_PATTERN.test(value))
    throw new HandoffError("soulsPerChip must be a decimal integer string");
  if (value === "0") throw new HandoffError("soulsPerChip must be positive");
  return value;
}

function requireContinuation(value) {
  requireString(value, "continuation");
  if (value.length !== CONTINUATION_LENGTH || !BASE64_URL_PATTERN.test(value)) {
    throw new HandoffError("continuation must be unpadded base64url for 32 bytes");
  }
  return value;
}

// ─── intent construction ───────────────────────────────────────────

/**
 * Build a validated intent object. Every kind carries the common fields; the
 * extra fields depend on the command. The result is ready for `encodeIntent`.
 *
 * @param {object} spec
 * @param {string} spec.kind - one of COMMAND_KIND
 * @param {{networkId:string, protocolVersion:string, genesisHash:string}} spec.scope
 * @param {string} spec.returnUrl - HTTPS or exact loopback origin on the game
 * @param {string} [spec.state] - opaque correlation value; generated if absent
 * @param {{uri:string, cid:string}} [spec.table] - PROPOSE only
 * @param {string} [spec.soulsPerChip] - PROPOSE only, decimal souls per chip
 * @param {{uri:string, cid:string}} [spec.contract] - FUND/ACTIVATE/WITHDRAW/SETTLE
 * @param {{uri:string, cid:string}} [spec.terminalAction] - SETTLE only
 * @returns {object} the validated intent
 */
export function buildIntent(spec) {
  if (!spec || typeof spec !== "object") throw new HandoffError("intent spec is required");
  const kind = spec.kind;
  if (!Object.values(COMMAND_KIND).includes(kind)) throw new HandoffError("Unknown command kind");

  const intent = {
    version: HANDOFF_VERSION,
    kind,
    scope: requireScope(spec.scope),
    returnUrl: requireReturnUrl(spec.returnUrl),
    state: spec.state == null ? newHandoffState() : requireState(spec.state),
  };

  switch (kind) {
    case COMMAND_KIND.PROPOSE:
      requireAbsent(spec, ["contract", "terminalAction"], kind);
      intent.table = requireStrongRef(spec.table, CARDCORE_COLLECTION.TABLE, "table");
      intent.soulsPerChip = requireSoulsPerChip(spec.soulsPerChip);
      break;
    case COMMAND_KIND.FUND:
    case COMMAND_KIND.ACTIVATE:
    case COMMAND_KIND.WITHDRAW:
      requireAbsent(spec, ["table", "soulsPerChip", "terminalAction"], kind);
      intent.contract = requireStrongRef(spec.contract, CONTRACT_COLLECTION.CONTRACT, "contract");
      break;
    case COMMAND_KIND.SETTLE:
      requireAbsent(spec, ["table", "soulsPerChip"], kind);
      intent.contract = requireStrongRef(spec.contract, CONTRACT_COLLECTION.CONTRACT, "contract");
      intent.terminalAction = requireStrongRef(
        spec.terminalAction,
        CARDCORE_COLLECTION.ACTION,
        "terminalAction",
      );
      break;
    default:
      throw new HandoffError("Unknown command kind");
  }
  return intent;
}

/** Encode a validated intent to the base64url JSON the wallet accepts. */
export function encodeIntent(intent) {
  const validated = buildIntent(intent);
  const encoded = stringToBase64Url(JSON.stringify(validated));
  if (encoded.length > MAX_ENCODED_INTENT_CHARS)
    throw new HandoffError("Encoded intent is too large");
  if (TEXT_ENCODER.encode(JSON.stringify(validated)).length > MAX_DECODED_INTENT_BYTES) {
    throw new HandoffError("Decoded intent is too large");
  }
  return encoded;
}

/** Decode a base64url intent, e.g. for a local self-check. */
export function decodeIntent(encoded) {
  requireString(encoded, "encoded intent");
  if (encoded.length > MAX_ENCODED_INTENT_CHARS)
    throw new HandoffError("Encoded intent is too large");
  const json = base64UrlToString(encoded);
  if (TEXT_ENCODER.encode(json).length > MAX_DECODED_INTENT_BYTES) {
    throw new HandoffError("Decoded intent is too large");
  }
  const parsed = JSON.parse(json);
  if (parsed.version !== HANDOFF_VERSION) throw new HandoffError("Unsupported handoff version");
  return buildIntent(parsed);
}

// ─── wallet URL ────────────────────────────────────────────────────

function normaliseWalletBase(walletBaseUrl) {
  const base = requireString(walletBaseUrl, "walletBaseUrl");
  let url;
  try {
    url = new URL(base);
  } catch {
    throw new HandoffError("walletBaseUrl is not a valid absolute URL");
  }
  const scheme = url.protocol.replace(/:$/, "").toLowerCase();
  const host = url.hostname.replace(/^\[/, "").replace(/\]$/, "").toLowerCase();
  if (scheme !== "https" && !(scheme === "http" && LOOPBACK_HOSTS.has(host))) {
    throw new HandoffError("walletBaseUrl must use HTTPS or exact loopback HTTP");
  }
  return base.replace(/\/+$/, "");
}

/**
 * Build the full-page handoff URL. For a continuation command the caller
 * passes a continuation token; it goes only in the URL fragment, never the
 * query, so the browser never sends it in a request URL or referrer.
 *
 * @param {string} walletBaseUrl - the operator wallet public origin
 * @param {object|string} intentOrEncoded - an intent object or an encoded intent
 * @param {string} [continuation] - continuation token for ACTIVATE/WITHDRAW/SETTLE
 * @returns {string}
 */
export function buildHandoffUrl(walletBaseUrl, intentOrEncoded, continuation) {
  const base = normaliseWalletBase(walletBaseUrl);
  const encoded =
    typeof intentOrEncoded === "string" ? intentOrEncoded : encodeIntent(intentOrEncoded);
  if (encoded.length > MAX_ENCODED_INTENT_CHARS)
    throw new HandoffError("Encoded intent is too large");
  let url = `${base}/wallet/cardcore/handoff?intent=${encoded}`;
  if (continuation != null) {
    url += `#continuation=${requireContinuation(continuation)}`;
  }
  return url;
}

// ─── receipt parsing ───────────────────────────────────────────────

function parsePublishedRecords(records) {
  if (records == null) return [];
  if (!Array.isArray(records)) throw new HandoffError("receipt records must be an array");
  const seen = new Set();
  return records.map((entry) => {
    if (!entry || typeof entry !== "object") throw new HandoffError("receipt record is invalid");
    const collection = requireString(entry.collection, "record collection");
    if (!PUBLISHED_COLLECTIONS.has(collection))
      throw new HandoffError("receipt record collection is invalid");
    const reference = requireStrongRef(entry.reference, collection, "record reference");
    const key = `${collection}|${reference.uri}|${reference.cid}`;
    if (seen.has(key)) throw new HandoffError("receipt records must be unique");
    seen.add(key);
    return { collection, reference };
  });
}

/** Validate and normalise a decoded receipt object. */
export function parseReceipt(raw) {
  if (!raw || typeof raw !== "object") throw new HandoffError("receipt must be an object");
  if (raw.version !== HANDOFF_VERSION) throw new HandoffError("Unsupported receipt version");
  const state = requireState(raw.state);
  const kind = raw.kind;
  if (!Object.values(COMMAND_KIND).includes(kind))
    throw new HandoffError("receipt kind is invalid");
  const status = raw.status;
  if (!RECEIPT_STATUSES.has(status)) throw new HandoffError("receipt status is invalid");
  const records = parsePublishedRecords(raw.records);
  const errorCode = raw.errorCode == null ? null : raw.errorCode;
  if (errorCode != null && !ERROR_CODES.has(errorCode))
    throw new HandoffError("receipt errorCode is invalid");
  const continuation = raw.continuation == null ? null : requireContinuation(raw.continuation);

  if (status === RECEIPT_STATUS.COMPLETE) {
    if (records.length === 0)
      throw new HandoffError("A complete receipt requires published records");
    if (errorCode != null) throw new HandoffError("A complete receipt cannot carry an error");
    if (continuation != null && TERMINAL_COMMANDS.has(kind)) {
      throw new HandoffError("A complete terminal receipt cannot carry a continuation");
    }
  } else {
    if (errorCode == null) throw new HandoffError(`A ${status} receipt requires an error code`);
    if (status === RECEIPT_STATUS.FAILED && continuation != null) {
      throw new HandoffError("A failed receipt cannot carry a continuation");
    }
  }
  return { version: HANDOFF_VERSION, state, kind, status, records, continuation, errorCode };
}

/** Decode a base64url receipt payload into a validated receipt. */
export function decodeReceipt(encoded) {
  requireString(encoded, "encoded receipt");
  return parseReceipt(JSON.parse(base64UrlToString(encoded)));
}

/**
 * Read a receipt from a return-URL fragment such as `#atbloons=<base64url>`.
 * Returns the validated receipt, or `null` when the fragment carries none.
 *
 * @param {string} hash - a `window.location.hash` value or bare fragment
 * @returns {object|null}
 */
export function readReceiptFragment(hash) {
  if (typeof hash !== "string" || hash.length === 0) return null;
  const fragment = hash.startsWith("#") ? hash.slice(1) : hash;
  for (const part of fragment.split("&")) {
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    if (part.slice(0, eq) !== "atbloons") continue;
    const value = part.slice(eq + 1);
    if (!value) return null;
    return decodeReceipt(decodeURIComponent(value));
  }
  return null;
}

export function isRetryable(receipt) {
  return receipt != null && receipt.status === RECEIPT_STATUS.RETRYABLE;
}

export function isComplete(receipt) {
  return receipt != null && receipt.status === RECEIPT_STATUS.COMPLETE;
}

export function isFailed(receipt) {
  return receipt != null && receipt.status === RECEIPT_STATUS.FAILED;
}

/** Find the first published reference for a collection, or null. */
export function publishedReference(receipt, collection) {
  if (!receipt || !Array.isArray(receipt.records)) return null;
  const match = receipt.records.find((record) => record.collection === collection);
  return match ? match.reference : null;
}
