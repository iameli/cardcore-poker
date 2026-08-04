/**
 * Unit tests for the paid-hand ChainedSession driver.
 *
 * These drive the transport/ordering logic with a deterministic FAKE engine so
 * the driver's behaviour is isolated: canonical global order, cross-author
 * `prev` linking, contiguous global `seq`, out-of-order peer buffering, the
 * betting pause, and the mandatory `verifySeed` tail. The REAL engine order is
 * proven separately (native Rust `chained_transcript` tests and the WASM
 * integration test in `chained-wasm.test.js`).
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { ChainedSession } from "./chained-driver.js";

/**
 * A tiny deterministic chained engine over a fixed script of canonical actions.
 * Each entry is { seat, kind }. The agent applies them strictly in order; a
 * received/produced action must equal the current slot's bytes, encoded as
 * [seat, index] so every slot is unique.
 */
class FakeChainedAgent {
  constructor(mySeat, script, options = {}) {
    this.mySeat = mySeat;
    this.script = script;
    this.applied = 0;
    this.options = options; // per-index bet options: { [index]: [...] }
    this.chained = false;
  }
  set_chained(on) {
    this.chained = on;
  }
  receive_table(_cbor) {}
  next_action() {
    if (this.applied >= this.script.length) return "null";
    const slot = this.script[this.applied];
    const out = { seat: slot.seat, kind: slot.kind, mine: slot.seat === this.mySeat };
    if (slot.kind === "bet") out.options = this.options[this.applied] || ["Check", "Fold"];
    return JSON.stringify(out);
  }
  _slotBytes(index) {
    const slot = this.script[index];
    return new Uint8Array([slot.seat, index]);
  }
  produce_next() {
    const slot = this.script[this.applied];
    if (slot.seat !== this.mySeat || slot.kind === "bet") return new Uint8Array();
    return this._slotBytes(this.applied);
  }
  produce_bet(_action) {
    return this._slotBytes(this.applied);
  }
  receive_action(cbor, _did) {
    const expected = this._slotBytes(this.applied);
    const got = cbor instanceof Uint8Array ? cbor : new Uint8Array(cbor);
    if (got.length !== expected.length || got[0] !== expected[0] || got[1] !== expected[1]) {
      throw new Error(`out-of-order: got [${got}], expected [${expected}]`);
    }
    this.applied += 1;
  }
  phase() {
    return this.applied >= this.script.length ? "Complete" : "Betting";
  }
}

/** A publish sink that records every published action and returns a fake ref. */
function recordingPublisher() {
  const published = [];
  const publish = async (args) => {
    const ref = { uri: `at://x/re.cardco.poker.action/t-${args.seq}`, cid: `cid-${args.seq}` };
    published.push({ ...args, ref });
    return ref;
  };
  return { published, publish };
}

const SCRIPT_2P = [
  { seat: 0, kind: "commitSeed" },
  { seat: 1, kind: "commitSeed" },
  { seat: 0, kind: "verifySeed" },
  { seat: 1, kind: "verifySeed" },
];

/** Peer bytes for a slot (what the peer's own produce would emit). */
function peerBytes(script, index) {
  return new Uint8Array([script[index].seat, index]);
}

test("drives seat 0 through a 2-player chain with global prev/seq linking", async () => {
  const agent = new FakeChainedAgent(0, SCRIPT_2P);
  const { published, publish } = recordingPublisher();
  const session = new ChainedSession({
    agent,
    did: "did:plc:seat0",
    tableRef: { uri: "at://x/re.cardco.poker.table/t", cid: "tcid" },
    tableTid: "t",
    publish,
  });

  await session.receiveTable(new Uint8Array());
  // Seat 0 publishes slot 0 (its commit), then waits for peer slot 1.
  assert.equal(published.length, 1);
  assert.equal(published[0].seq, 0);
  assert.equal(published[0].prevRef, null);
  assert.equal(session.nextSeq, 1);

  // Peer slot 1 arrives.
  await session.receiveChainAction(peerBytes(SCRIPT_2P, 1), {
    did: "did:plc:seat1",
    seq: 1,
    ref: { uri: "at://y/re.cardco.poker.action/t-1", cid: "cid-1" },
  });
  // Now seat 0 can publish slot 2, then waits for peer slot 3.
  assert.equal(published.length, 2);
  assert.equal(published[1].seq, 2);
  // The published prev must point at the peer's slot-1 record (cross-author link).
  assert.deepEqual(published[1].prevRef, {
    uri: "at://y/re.cardco.poker.action/t-1",
    cid: "cid-1",
  });

  // Peer slot 3 (final seed reveal) arrives → chain completes.
  await session.receiveChainAction(peerBytes(SCRIPT_2P, 3), {
    did: "did:plc:seat1",
    seq: 3,
    ref: { uri: "at://y/re.cardco.poker.action/t-3", cid: "cid-3" },
  });

  assert.equal(session.isComplete, true);
  assert.equal(session.nextSeq, 4);
  // The full chain is contiguous, seat-ordered, and ends with both seed reveals.
  assert.deepEqual(
    session.chain.map((c) => c.seq),
    [0, 1, 2, 3],
  );
  assert.deepEqual(
    session.chain.map((c) => c.seat),
    [0, 1, 0, 1],
  );
  const tail = session.chain.slice(-2);
  assert.ok(tail.every((c) => c.kind === "verifySeed"));
});

test("buffers out-of-order peer actions until their slot is reached", async () => {
  const agent = new FakeChainedAgent(0, SCRIPT_2P);
  const { publish } = recordingPublisher();
  const session = new ChainedSession({
    agent,
    did: "did:plc:seat0",
    tableRef: { uri: "at://x/t", cid: "tcid" },
    tableTid: "t",
    publish,
  });
  await session.receiveTable(new Uint8Array());

  // Deliver the LAST peer slot (3) first — must buffer, not apply out of order.
  await session.receiveChainAction(peerBytes(SCRIPT_2P, 3), {
    did: "did:plc:seat1",
    seq: 3,
    ref: { uri: "at://y/t-3", cid: "cid-3" },
  });
  assert.equal(session.isComplete, false);
  assert.equal(session.nextSeq, 1); // still waiting on slot 1

  // Now deliver slot 1 — that unblocks slot 2 (ours) and then the buffered slot 3.
  await session.receiveChainAction(peerBytes(SCRIPT_2P, 1), {
    did: "did:plc:seat1",
    seq: 1,
    ref: { uri: "at://y/t-1", cid: "cid-1" },
  });
  assert.equal(session.isComplete, true);
  assert.equal(session.nextSeq, 4);
});

test("pauses for a human bet, then produces and publishes it", async () => {
  const script = [
    { seat: 0, kind: "commitSeed" },
    { seat: 0, kind: "bet" },
    { seat: 0, kind: "verifySeed" },
  ];
  const agent = new FakeChainedAgent(0, script, { 1: ["Check", "Fold"] });
  const { published, publish } = recordingPublisher();
  const session = new ChainedSession({
    agent,
    did: "did:plc:seat0",
    tableRef: { uri: "at://x/t", cid: "tcid" },
    tableTid: "t",
    publish,
  });

  await session.receiveTable(new Uint8Array());
  // Stopped at the bet, awaiting a decision.
  assert.equal(session.needsBet, true);
  assert.deepEqual(session.betOptions, ["Check", "Fold"]);
  assert.equal(published.length, 1); // only the commit so far

  await session.submitBet("check");
  assert.equal(session.needsBet, false);
  assert.equal(session.isComplete, true);
  // commit(0), bet(1), verifySeed(2) all published, contiguous seq.
  assert.deepEqual(
    published.map((p) => p.seq),
    [0, 1, 2],
  );
});

test("resumes a mine slot from a delivered own-echo instead of re-prompting", async () => {
  // A reloaded paid hand replays our OWN repo on the firehose. When our own
  // past bet slot arrives, the driver must apply it in place — not prompt for
  // a bet we already published — and must not re-publish it.
  const script = [
    { seat: 0, kind: "commitSeed" },
    { seat: 0, kind: "bet" },
    { seat: 0, kind: "verifySeed" },
  ];
  const agent = new FakeChainedAgent(0, script, { 1: ["Check", "Fold"] });
  const { published, publish } = recordingPublisher();
  const session = new ChainedSession({
    agent,
    did: "did:plc:seat0",
    tableRef: { uri: "at://x/t", cid: "tcid" },
    tableTid: "t",
    publish,
  });

  await session.receiveTable(new Uint8Array());
  // Commit (slot 0) republished; paused at our bet (slot 1), echo not yet in.
  assert.equal(session.needsBet, true);
  assert.equal(session.nextSeq, 1);
  assert.deepEqual(
    published.map((p) => p.seq),
    [0],
  );

  // Our own past bet arrives on the firehose (resume replay).
  await session.receiveChainAction(peerBytes(script, 1), {
    did: "did:plc:seat0",
    seq: 1,
    ref: { uri: "at://x/re.cardco.poker.action/t-1", cid: "cid-1" },
  });

  // The prompt cleared, the chain finished, and the bet was NOT re-published:
  // only the commit and the auto-produced verifySeed came from us.
  assert.equal(session.needsBet, false);
  assert.equal(session.isComplete, true);
  assert.equal(session.nextSeq, 3);
  assert.deepEqual(
    published.map((p) => p.seq),
    [0, 2],
  );
  // The applied bet carries the delivered echo's strong ref as its chain link.
  const betSlot = session.chain.find((c) => c.seq === 1);
  assert.equal(betSlot.kind, "bet");
  assert.deepEqual(betSlot.ref, { uri: "at://x/re.cardco.poker.action/t-1", cid: "cid-1" });
});

test("validates constructor arguments (fail closed)", () => {
  assert.throws(() => new ChainedSession({ did: "d", publish: () => {} }), /requires an agent/);
  assert.throws(() => new ChainedSession({ agent: {}, publish: () => {} }), /requires a did/);
  assert.throws(() => new ChainedSession({ agent: {}, did: "d" }), /requires a publish/);
});

test("ignores stale echoes of already-applied slots", async () => {
  const agent = new FakeChainedAgent(0, SCRIPT_2P);
  const { publish } = recordingPublisher();
  const session = new ChainedSession({
    agent,
    did: "did:plc:seat0",
    tableRef: { uri: "at://x/t", cid: "tcid" },
    tableTid: "t",
    publish,
  });
  await session.receiveTable(new Uint8Array());
  await session.receiveChainAction(peerBytes(SCRIPT_2P, 1), {
    did: "did:plc:seat1",
    seq: 1,
    ref: { uri: "at://y/t-1", cid: "cid-1" },
  });
  // Re-deliver slot 0 (our own, long applied) — must be ignored, not reapplied.
  await session.receiveChainAction(peerBytes(SCRIPT_2P, 0), {
    did: "did:plc:seat0",
    seq: 0,
    ref: { uri: "at://x/t-0", cid: "cid-0" },
  });
  assert.equal(session.nextSeq, 3); // unchanged by the stale echo
});
