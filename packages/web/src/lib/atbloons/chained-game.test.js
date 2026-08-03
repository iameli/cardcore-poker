/**
 * Unit tests for the paid-hand live-wiring helpers in chained-game.js:
 * content-addressed strong refs, the rkey scheme, and the Publisher→driver
 * publish wrapper. The full chain (real refs + real CIDs through the compiled
 * engine) is proven in chained-wasm.test.js.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ChainedGame,
  actionRkey,
  actionStrongRef,
  computeRecordCid,
  makeChainedPublish,
} from "./chained-game.js";

/**
 * A minimal fake WASM agent exposing only the read-state surface the UI needs.
 * `set_chained` is required because ChainedSession calls it in its constructor.
 */
function fakeAgent(state) {
  return {
    set_chained() {},
    hole_cards: () => JSON.stringify(state.hole || []),
    community_cards: () => JSON.stringify(state.community || []),
    phase: () => state.phase || "Betting",
    game_state: () => JSON.stringify(state.gameState ?? null),
    waiting_on: () => JSON.stringify(state.waiting || []),
    last_hand_result: () => (state.result ? JSON.stringify(state.result) : ""),
    game_over: () => !!state.over,
    free() {
      state._freed = true;
    },
  };
}

const RECORD = {
  $type: "re.cardco.poker.action",
  table: { uri: "at://did:plc:host/re.cardco.poker.table/t", cid: "tablecid" },
  seq: 3,
  action: { $type: "re.cardco.poker.defs#commitSeed", commitment: new Uint8Array([1, 2, 3, 4]) },
  createdAt: "2026-07-31T00:00:00.000Z",
};

test("action rkeys zero-pad the global seq to 9 digits", () => {
  assert.equal(actionRkey("t", 0), "t-000000000");
  assert.equal(actionRkey("t", 42), "t-000000042");
  assert.throws(() => actionRkey("t", 1_000_000_000), /exceeds rkey padding/);
});

test("computeRecordCid is deterministic, content-addressed, and well-formed", async () => {
  const a = await computeRecordCid(RECORD);
  const b = await computeRecordCid({ ...RECORD });
  assert.equal(a, b, "same record → same CID");
  // DASL CIDv1 dag-cbor sha-256: multibase 'b' + base32 of 36 bytes.
  assert.match(a, /^b[a-z2-7]+$/);
  assert.equal(a.length, 59);

  const changed = await computeRecordCid({ ...RECORD, seq: 4 });
  assert.notEqual(a, changed, "different content → different CID");
});

test("actionStrongRef builds an at:// uri with the padded rkey and the record CID", async () => {
  const ref = await actionStrongRef({
    did: "did:plc:host",
    tableTid: "t",
    seq: 3,
    record: RECORD,
  });
  assert.equal(ref.uri, "at://did:plc:host/re.cardco.poker.action/t-000000003");
  assert.equal(ref.cid, await computeRecordCid(RECORD));
});

test("makeChainedPublish forwards the action cbor to onTip for logging", async () => {
  const publisher = { publishAction: async () => ({ uri: "u", cid: "c" }) };
  const seen = [];
  const publish = makeChainedPublish({
    publisher,
    onTip: (ref, seq, cbor) => seen.push({ ref, seq, cbor }),
  });
  const cbor = new Uint8Array([1, 2, 3]);
  await publish({ tableRef: {}, prevRef: null, seq: 7, tableTid: "t", actionCbor: cbor });
  assert.equal(seen.length, 1);
  assert.equal(seen[0].seq, 7);
  assert.deepEqual(seen[0].cbor, cbor);
});

test("ChainedGame exposes agent-backed read state and frees the agent on destroy", () => {
  const state = {
    hole: ["As", "Kd"],
    community: ["2c", "3h", "4s"],
    phase: "Flop",
    gameState: { pot: 30, players: [] },
    waiting: [{ kind: "bet", seats: [1] }],
    result: { hand_index: 0 },
    over: false,
  };
  const game = new ChainedGame({
    agent: fakeAgent(state),
    did: "did:plc:seat0",
    tableRef: { uri: "at://x/t", cid: "tc" },
    tableTid: "t",
    publisher: { publishAction: async () => ({ uri: "u", cid: "c" }) },
  });
  // The UI reads the SAME surface it reads from PlayerSession.
  assert.deepEqual(game.rawHoleCards, ["As", "Kd"]);
  assert.deepEqual(game.rawCommunityCards, ["2c", "3h", "4s"]);
  assert.equal(game.phase, "Flop");
  assert.deepEqual(game.gameState, { pot: 30, players: [] });
  assert.deepEqual(game.waitingOn, [{ kind: "bet", seats: [1] }]);
  assert.deepEqual(game.lastHandResult, { hand_index: 0 });
  assert.equal(game.gameOver, false);
  assert.equal(game.pendingCount, 0);
  assert.equal(game.needsBet, false);
  assert.equal(game.tipRef, null);

  game.destroy();
  assert.equal(state._freed, true);
});

test("makeChainedPublish forwards to the publisher, returns the ref, and reports the tip", async () => {
  const calls = [];
  const publisher = {
    publishAction: async (args) => {
      calls.push(args);
      return { uri: "at://did:plc:host/re.cardco.poker.action/t-000000005", cid: "cid-5" };
    },
  };
  const tips = [];
  const publish = makeChainedPublish({ publisher, onTip: (ref, seq) => tips.push({ ref, seq }) });

  const cbor = new Uint8Array([9, 9]);
  const ref = await publish({
    tableRef: { uri: "at://t", cid: "tc" },
    prevRef: { uri: "at://p", cid: "pc" },
    seq: 5,
    tableTid: "t",
    actionCbor: cbor,
  });

  assert.deepEqual(ref, {
    uri: "at://did:plc:host/re.cardco.poker.action/t-000000005",
    cid: "cid-5",
  });
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].prevRef, { uri: "at://p", cid: "pc" });
  assert.equal(calls[0].seq, 5);
  assert.deepEqual(tips, [{ ref, seq: 5 }]);
});
