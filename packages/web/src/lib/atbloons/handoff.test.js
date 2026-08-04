/**
 * Offline unit tests for the atbloons handoff client contract.
 * Run with: node --test packages/web/src/lib/atbloons/handoff.test.js
 * No network and no external dependency.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  COMMAND_KIND,
  CARDCORE_COLLECTION,
  CONTRACT_COLLECTION,
  HANDOFF_VERSION,
  HandoffError,
  base64UrlToBytes,
  buildHandoffUrl,
  buildIntent,
  bytesToBase64Url,
  decodeIntent,
  decodeReceipt,
  encodeIntent,
  newHandoffState,
  parseReceipt,
  publishedReference,
  readReceiptFragment,
} from "./handoff.js";

const SCOPE = {
  networkId: "atbloons-cardcore-testnet",
  protocolVersion: "v3",
  genesisHash: "a".repeat(64),
};
const RETURN_URL = "https://game.example/callback";
const STATE = bytesToBase64Url(new Uint8Array(16).map((_, i) => i)); // deterministic 16-byte state
const TABLE_REF = { uri: "at://did:plc:alice/re.cardco.poker.table/table", cid: "bafyTable" };
const CONTRACT_REF = {
  uri: "at://did:plc:alice/tech.lenooby09.atbloons.contract/contract",
  cid: "bafyContract",
};
const ACTION_REF = { uri: "at://did:plc:alice/re.cardco.poker.action/terminal", cid: "bafyAction" };
const CONTINUATION = bytesToBase64Url(new Uint8Array(32).map((_, i) => i));

function proposeIntent(overrides = {}) {
  return {
    kind: COMMAND_KIND.PROPOSE,
    scope: SCOPE,
    returnUrl: RETURN_URL,
    state: STATE,
    table: TABLE_REF,
    soulsPerChip: "9223372036854775807",
    ...overrides,
  };
}

// ─── base64url primitives ──────────────────────────────────────────

test("base64url round trips arbitrary bytes", () => {
  for (let len = 0; len <= 40; len++) {
    const bytes = new Uint8Array(len).map((_, i) => (i * 37 + 11) & 255);
    const encoded = bytesToBase64Url(bytes);
    assert.equal(/^[A-Za-z0-9_-]*$/.test(encoded), true);
    if (len > 0) assert.deepEqual([...base64UrlToBytes(encoded)], [...bytes]);
  }
});

test("base64url decode rejects padded or non-alphabet input", () => {
  assert.throws(() => base64UrlToBytes("AAAA=="), HandoffError);
  assert.throws(() => base64UrlToBytes("****"), HandoffError);
});

test("a fresh handoff state is 16 bytes and 22 base64url characters", () => {
  const state = newHandoffState();
  assert.equal(state.length, 22);
  assert.equal(base64UrlToBytes(state).length, 16);
});

// ─── intent construction and wire shape ────────────────────────────

test("builds and round trips all five valid intents", () => {
  const intents = [
    proposeIntent(),
    {
      kind: COMMAND_KIND.FUND,
      scope: SCOPE,
      returnUrl: RETURN_URL,
      state: STATE,
      contract: CONTRACT_REF,
    },
    {
      kind: COMMAND_KIND.ACTIVATE,
      scope: SCOPE,
      returnUrl: RETURN_URL,
      state: STATE,
      contract: CONTRACT_REF,
    },
    {
      kind: COMMAND_KIND.WITHDRAW,
      scope: SCOPE,
      returnUrl: RETURN_URL,
      state: STATE,
      contract: CONTRACT_REF,
    },
    {
      kind: COMMAND_KIND.SETTLE,
      scope: SCOPE,
      returnUrl: RETURN_URL,
      state: STATE,
      contract: CONTRACT_REF,
      terminalAction: ACTION_REF,
    },
  ];
  for (const spec of intents) {
    const encoded = encodeIntent(spec);
    const decoded = decodeIntent(encoded);
    assert.equal(decoded.kind, spec.kind);
    assert.equal(decoded.version, HANDOFF_VERSION);
    assert.deepEqual(decoded.scope, SCOPE);
  }
});

test("encoded PROPOSE intent has the exact atbloons wire JSON", () => {
  const json = JSON.parse(Buffer.from(encodeIntent(proposeIntent()), "base64url").toString("utf8"));
  assert.deepEqual(json, {
    version: "cardcore-poker-contract-v1",
    kind: "PROPOSE",
    scope: SCOPE,
    returnUrl: RETURN_URL,
    state: STATE,
    table: TABLE_REF,
    soulsPerChip: "9223372036854775807",
  });
});

test("uppercase command kinds match the atbloons enum on the wire", () => {
  for (const kind of ["PROPOSE", "FUND", "ACTIVATE", "WITHDRAW", "SETTLE"]) {
    assert.equal(COMMAND_KIND[kind], kind);
  }
});

// ─── validation mirrors the atbloons DTO ───────────────────────────

test("rejects an unsupported protocol version", () => {
  assert.throws(
    () => buildIntent(proposeIntent({ scope: { ...SCOPE, protocolVersion: "v2" } })),
    HandoffError,
  );
});

test("rejects a mismatched command payload", () => {
  assert.throws(() => buildIntent(proposeIntent({ contract: CONTRACT_REF })), HandoffError);
  assert.throws(
    () =>
      buildIntent({
        kind: COMMAND_KIND.FUND,
        scope: SCOPE,
        returnUrl: RETURN_URL,
        state: STATE,
        table: TABLE_REF,
      }),
    HandoffError,
  );
  assert.throws(
    () =>
      buildIntent({
        kind: COMMAND_KIND.SETTLE,
        scope: SCOPE,
        returnUrl: RETURN_URL,
        state: STATE,
        contract: CONTRACT_REF,
      }),
    HandoffError,
  );
});

test("rejects a strong reference from the wrong collection", () => {
  assert.throws(() => buildIntent(proposeIntent({ table: CONTRACT_REF })), HandoffError);
  assert.throws(
    () =>
      buildIntent({
        kind: COMMAND_KIND.FUND,
        scope: SCOPE,
        returnUrl: RETURN_URL,
        state: STATE,
        contract: TABLE_REF,
      }),
    HandoffError,
  );
});

test("rejects a non-positive souls-per-chip", () => {
  assert.throws(() => buildIntent(proposeIntent({ soulsPerChip: "0" })), HandoffError);
  assert.throws(() => buildIntent(proposeIntent({ soulsPerChip: "-1" })), HandoffError);
  assert.throws(() => buildIntent(proposeIntent({ soulsPerChip: "1.5" })), HandoffError);
});

test("rejects unsafe return URLs and accepts loopback HTTP", () => {
  for (const bad of [
    "http://game.example/callback", // non-loopback HTTP
    "https://game.example/callback#frag", // fragment
    "https://user@game.example/callback", // userinfo
    "ftp://game.example/callback", // scheme
    "not-a-url",
  ]) {
    assert.throws(() => buildIntent(proposeIntent({ returnUrl: bad })), HandoffError, bad);
  }
  assert.equal(
    buildIntent(proposeIntent({ returnUrl: "http://127.0.0.1:5173/return" })).returnUrl,
    "http://127.0.0.1:5173/return",
  );
});

test("rejects a state that is too short", () => {
  const shortState = bytesToBase64Url(new Uint8Array(8));
  assert.throws(() => buildIntent(proposeIntent({ state: shortState })), HandoffError);
});

// ─── wallet URL ────────────────────────────────────────────────────

test("builds a handoff URL with the intent in the query", () => {
  const url = buildHandoffUrl("https://wallet.example/", proposeIntent());
  assert.match(
    url,
    /^https:\/\/wallet\.example\/wallet\/cardcore\/handoff\?intent=[A-Za-z0-9_-]+$/,
  );
  assert.equal(url.includes("#"), false);
});

test("puts a continuation token only in the fragment", () => {
  const url = buildHandoffUrl(
    "https://wallet.example",
    {
      kind: COMMAND_KIND.ACTIVATE,
      scope: SCOPE,
      returnUrl: RETURN_URL,
      state: STATE,
      contract: CONTRACT_REF,
    },
    CONTINUATION,
  );
  const [beforeHash, afterHash] = url.split("#");
  assert.equal(beforeHash.includes(CONTINUATION), false);
  assert.equal(afterHash, `continuation=${CONTINUATION}`);
});

test("rejects an insecure wallet base URL", () => {
  assert.throws(() => buildHandoffUrl("http://wallet.example", proposeIntent()), HandoffError);
});

// ─── receipt parsing ───────────────────────────────────────────────

function completeReceipt(overrides = {}) {
  return {
    version: HANDOFF_VERSION,
    state: STATE,
    kind: COMMAND_KIND.PROPOSE,
    status: "complete",
    records: [{ collection: CONTRACT_COLLECTION.CONTRACT, reference: CONTRACT_REF }],
    continuation: CONTINUATION,
    ...overrides,
  };
}

test("parses a complete receipt and exposes published references", () => {
  const receipt = parseReceipt(completeReceipt());
  assert.equal(receipt.status, "complete");
  assert.deepEqual(publishedReference(receipt, CONTRACT_COLLECTION.CONTRACT), CONTRACT_REF);
});

test("rejects a complete receipt with no records", () => {
  assert.throws(() => parseReceipt(completeReceipt({ records: [] })), HandoffError);
});

test("rejects a retryable or failed receipt with no error code", () => {
  assert.throws(
    () => parseReceipt(completeReceipt({ status: "retryable", records: [], continuation: null })),
    HandoffError,
  );
  assert.throws(
    () => parseReceipt(completeReceipt({ status: "failed", records: [], continuation: null })),
    HandoffError,
  );
});

test("rejects a failed receipt that carries a continuation", () => {
  assert.throws(
    () =>
      parseReceipt(
        completeReceipt({ status: "failed", records: [], errorCode: "cardcore_invalid" }),
      ),
    HandoffError,
  );
});

test("rejects a complete terminal receipt that carries a continuation", () => {
  assert.throws(
    () =>
      parseReceipt(
        completeReceipt({
          kind: COMMAND_KIND.SETTLE,
          records: [
            {
              collection: CONTRACT_COLLECTION.SETTLEMENT,
              reference: {
                uri: "at://did:plc:alice/tech.lenooby09.atbloons.contractSettlement/s",
                cid: "c",
              },
            },
          ],
        }),
      ),
    HandoffError,
  );
});

test("rejects duplicate published records", () => {
  assert.throws(
    () =>
      parseReceipt(
        completeReceipt({
          continuation: null,
          records: [
            { collection: CONTRACT_COLLECTION.CONTRACT, reference: CONTRACT_REF },
            { collection: CONTRACT_COLLECTION.CONTRACT, reference: CONTRACT_REF },
          ],
        }),
      ),
    HandoffError,
  );
});

test("reads a receipt from a return fragment and ignores others", () => {
  const encoded = Buffer.from(
    JSON.stringify(completeReceipt({ continuation: null })),
    "utf8",
  ).toString("base64url");
  assert.equal(readReceiptFragment(""), null);
  assert.equal(readReceiptFragment("#other=1"), null);
  const receipt = readReceiptFragment(`#foo=bar&atbloons=${encoded}`);
  assert.equal(receipt.status, "complete");
  assert.deepEqual(decodeReceipt(encoded).records, receipt.records);
});
