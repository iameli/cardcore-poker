/**
 * Offline lifecycle tests for the paid-hand controller.
 * Run with: node --test packages/web/src/lib/atbloons/paid-hand.test.js
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { bytesToBase64Url, COMMAND_KIND, CONTRACT_COLLECTION, HANDOFF_VERSION } from "./handoff.js";
import { GRANT_STATE, PaidHandController, SEAT_ROLE } from "./paid-hand.js";

const SCOPE = {
  networkId: "atbloons-cardcore-testnet",
  protocolVersion: "v3",
  genesisHash: "a".repeat(64),
};
const RETURN_URL = "https://game.example/return";
const WALLET = "https://wallet.example";
const TABLE_REF = { uri: "at://did:plc:host/re.cardco.poker.table/t1", cid: "bafyTable" };
const CONTRACT_REF = {
  uri: "at://did:plc:host/tech.lenooby09.atbloons.contract/c1",
  cid: "bafyContract",
};
const ACTION_REF = { uri: "at://did:plc:host/re.cardco.poker.action/terminal", cid: "bafyAction" };

const continuation = (byte) => bytesToBase64Url(new Uint8Array(32).fill(byte));
const publishedContract = { collection: CONTRACT_COLLECTION.CONTRACT, reference: CONTRACT_REF };
const publishedActivation = {
  collection: CONTRACT_COLLECTION.ACTIVATION,
  reference: {
    uri: "at://did:plc:host/tech.lenooby09.atbloons.contractActivation/a1",
    cid: "bafyAct",
  },
};
const publishedSettlement = {
  collection: CONTRACT_COLLECTION.SETTLEMENT,
  reference: {
    uri: "at://did:plc:host/tech.lenooby09.atbloons.contractSettlement/s1",
    cid: "bafySettle",
  },
};
const publishedFunding = {
  collection: CONTRACT_COLLECTION.FUNDING,
  reference: {
    uri: "at://did:plc:bob/tech.lenooby09.atbloons.contractFunding/f1",
    cid: "bafyFund",
  },
};
const publishedWithdrawal = {
  collection: CONTRACT_COLLECTION.WITHDRAWAL,
  reference: {
    uri: "at://did:plc:bob/tech.lenooby09.atbloons.contractWithdrawal/w1",
    cid: "bafyWd",
  },
};

function mapStore() {
  const map = new Map();
  return {
    map,
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => map.set(k, String(v)),
    removeItem: (k) => map.delete(k),
  };
}

/** Deterministic state generator that hands out distinct valid 16-byte states. */
function stateSource() {
  let n = 1;
  const issued = [];
  const fn = () => {
    const state = bytesToBase64Url(new Uint8Array(16).fill(n++));
    issued.push(state);
    return state;
  };
  fn.last = () => issued[issued.length - 1];
  return fn;
}

function controller(storage, newState) {
  return new PaidHandController({
    walletBaseUrl: WALLET,
    scope: SCOPE,
    returnUrl: RETURN_URL,
    storage,
    newState,
  });
}

function receipt(kind, status, extra = {}) {
  return {
    version: HANDOFF_VERSION,
    kind,
    status,
    records: [],
    continuation: null,
    errorCode: null,
    ...extra,
  };
}

test("host drives propose to activate to settle", () => {
  const store = mapStore();
  const states = stateSource();
  const host = controller(store, states);

  const proposeUrl = host.startProposal({ table: TABLE_REF, soulsPerChip: "1000" });
  assert.match(proposeUrl, /\/wallet\/cardcore\/handoff\?intent=/);
  assert.equal(proposeUrl.includes("#"), false);

  host.applyReceipt(
    receipt(COMMAND_KIND.PROPOSE, "complete", {
      state: states.last(),
      records: [publishedContract],
      continuation: continuation(1),
    }),
  );
  assert.equal(host.grant, GRANT_STATE.ACTIVE);
  assert.deepEqual(host.contract, CONTRACT_REF);
  assert.equal(host.canActivate, true);

  const activateUrl = host.startActivation();
  assert.match(activateUrl, new RegExp(`#continuation=${continuation(1)}$`));

  host.applyReceipt(
    receipt(COMMAND_KIND.ACTIVATE, "complete", {
      state: states.last(),
      records: [publishedActivation],
      continuation: continuation(2),
    }),
  );
  assert.equal(host.canSettle, true);

  const settleUrl = host.startSettlement({ terminalAction: ACTION_REF });
  assert.match(settleUrl, new RegExp(`#continuation=${continuation(2)}$`));

  host.applyReceipt(
    receipt(COMMAND_KIND.SETTLE, "complete", {
      state: states.last(),
      records: [publishedSettlement],
    }),
  );
  assert.equal(host.grant, GRANT_STATE.SETTLED);
  assert.equal(host.isClosed, true);
});

test("participant funds then withdraws before activation", () => {
  const store = mapStore();
  const states = stateSource();
  const bob = controller(store, states);

  bob.startFunding({ contract: CONTRACT_REF });
  bob.applyReceipt(
    receipt(COMMAND_KIND.FUND, "complete", {
      state: states.last(),
      records: [publishedFunding],
      continuation: continuation(5),
    }),
  );
  assert.equal(bob.state.role, SEAT_ROLE.PARTICIPANT);
  assert.equal(bob.canWithdraw, true);

  bob.startWithdrawal();
  bob.applyReceipt(
    receipt(COMMAND_KIND.WITHDRAW, "complete", {
      state: states.last(),
      records: [publishedWithdrawal],
    }),
  );
  assert.equal(bob.grant, GRANT_STATE.WITHDRAWN);
  assert.equal(bob.isClosed, true);
});

test("rejects a receipt whose state does not echo the request", () => {
  const store = mapStore();
  const states = stateSource();
  const host = controller(store, states);
  host.startProposal({ table: TABLE_REF, soulsPerChip: "1000" });
  assert.throws(
    () =>
      host.applyReceipt(
        receipt(COMMAND_KIND.PROPOSE, "complete", {
          state: bytesToBase64Url(new Uint8Array(16).fill(99)),
          records: [publishedContract],
        }),
      ),
    /state does not match/,
  );
});

test("rejects a receipt whose kind does not match the request", () => {
  const store = mapStore();
  const states = stateSource();
  const host = controller(store, states);
  host.startProposal({ table: TABLE_REF, soulsPerChip: "1000" });
  assert.throws(
    () =>
      host.applyReceipt(
        receipt(COMMAND_KIND.FUND, "complete", {
          state: states.last(),
          records: [publishedFunding],
        }),
      ),
    /kind does not match/,
  );
});

test("a retryable receipt keeps the pending step so the same intent can retry", () => {
  const store = mapStore();
  const states = stateSource();
  const host = controller(store, states);
  host.startProposal({ table: TABLE_REF, soulsPerChip: "1000" });
  const firstState = states.last();
  host.applyReceipt(
    receipt(COMMAND_KIND.PROPOSE, "retryable", {
      state: firstState,
      errorCode: "publication_pending",
    }),
  );
  assert.equal(host.state.pendingState, firstState);
  assert.equal(host.grant, GRANT_STATE.ACTIVE);

  // Retry succeeds with the same pending state.
  host.applyReceipt(
    receipt(COMMAND_KIND.PROPOSE, "complete", {
      state: firstState,
      records: [publishedContract],
      continuation: continuation(1),
    }),
  );
  assert.deepEqual(host.contract, CONTRACT_REF);
});

test("a contested contract failure revokes the grant", () => {
  const store = mapStore();
  const states = stateSource();
  const host = controller(store, states);
  host.startProposal({ table: TABLE_REF, soulsPerChip: "1000" });
  host.applyReceipt(
    receipt(COMMAND_KIND.PROPOSE, "complete", {
      state: states.last(),
      records: [publishedContract],
      continuation: continuation(1),
    }),
  );
  host.startActivation();
  host.applyReceipt(
    receipt(COMMAND_KIND.ACTIVATE, "failed", {
      state: states.last(),
      errorCode: "contract_terminal",
    }),
  );
  assert.equal(host.grant, GRANT_STATE.REVOKED);
  assert.equal(host.isClosed, true);
});

test("state survives a full-page reload through storage", () => {
  const store = mapStore();
  const states = stateSource();
  const host = controller(store, states);
  host.startProposal({ table: TABLE_REF, soulsPerChip: "1000" });
  const pending = states.last();

  // Simulate a redirect back into a fresh controller sharing the same storage.
  const resumed = controller(store, states);
  resumed.applyReceipt(
    receipt(COMMAND_KIND.PROPOSE, "complete", {
      state: pending,
      records: [publishedContract],
      continuation: continuation(1),
    }),
  );
  assert.deepEqual(resumed.contract, CONTRACT_REF);
  assert.equal(resumed.canActivate, true);
});

test("a closed grant refuses to start another hand", () => {
  const store = mapStore();
  const states = stateSource();
  const host = controller(store, states);
  host.startProposal({ table: TABLE_REF, soulsPerChip: "1000" });
  host.applyReceipt(
    receipt(COMMAND_KIND.PROPOSE, "complete", {
      state: states.last(),
      records: [publishedContract],
      continuation: continuation(1),
    }),
  );
  host.startActivation();
  host.applyReceipt(
    receipt(COMMAND_KIND.ACTIVATE, "complete", {
      state: states.last(),
      records: [publishedActivation],
      continuation: continuation(2),
    }),
  );
  host.startSettlement({ terminalAction: ACTION_REF });
  host.applyReceipt(
    receipt(COMMAND_KIND.SETTLE, "complete", {
      state: states.last(),
      records: [publishedSettlement],
    }),
  );
  assert.throws(
    () => host.startProposal({ table: TABLE_REF, soulsPerChip: "1000" }),
    /already closed/,
  );
});

test("consumeReturn parses an atbloons fragment and applies it", () => {
  const store = mapStore();
  const states = stateSource();
  const host = controller(store, states);
  host.startProposal({ table: TABLE_REF, soulsPerChip: "1000" });
  const payload = receipt(COMMAND_KIND.PROPOSE, "complete", {
    state: states.last(),
    records: [publishedContract],
    continuation: continuation(1),
  });
  const encoded = Buffer.from(JSON.stringify(payload), "utf8").toString("base64url");
  const applied = host.consumeReturn(`#atbloons=${encoded}`);
  assert.equal(applied.status, "complete");
  assert.deepEqual(host.contract, CONTRACT_REF);
});
