/**
 * Cross-layer integration: drives the REAL compiled engine (Rust → WASM,
 * `pkg-node`) through the paid-hand `ChainedGame`/`ChainedSession` over an
 * in-memory bus, publishing REAL `re.cardco.poker.action` records with REAL
 * DASL CIDs. It proves the seats build ONE contiguous, globally ordered chain
 * where every record's `prev` is the content-addressed strong ref of the
 * previous action (often a DIFFERENT author's repo) and the tail is every
 * seat's `verifySeed` — settlement-valid for the atbloons `CARDCORE_POKER_V1`
 * evaluator.
 *
 * Complements the native Rust `chained_transcript` proof (engine order) and the
 * fake-agent `chained-driver.test.js` (transport logic). Self-skips when
 * `pkg-node` (`just build-wasm-node`) or `@ipld/dag-cbor` is absent, so the
 * always-on offline suite stays green without a WASM build step.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { ChainedGame, actionRkey, computeRecordCid } from "./chained-game.js";

const require = createRequire(import.meta.url);
const ACTION = "re.cardco.poker.action";
const FIXED_CREATED_AT = "2026-07-31T00:00:00.000Z";

async function loadDeps() {
	try {
		const wasm = require("../../../../../pkg-node/cardcore_poker.js");
		const dagCbor = await import("@ipld/dag-cbor");
		return { WasmAgent: wasm.WasmAgent, dagCbor };
	} catch (err) {
		return { skip: String(err && err.message ? err.message : err) };
	}
}

function passiveBet(options) {
	if (options.includes("Check")) return "check";
	if (options.includes("Call")) return "call";
	return "fold";
}

/**
 * A publisher that builds a REAL action record (data-model form), computes its
 * real DASL CID, records it, and pushes it on the shared bus for peers. This is
 * exactly the record shape the PDS stores and the atbloons node re-verifies.
 */
function realRecordPublisher(dagCbor, did, tableTid, bus, published) {
	return {
		publishAction: async ({ tableRef, prevRef, seq, actionCbor }) => {
			const record = {
				$type: ACTION,
				table: tableRef,
				seq,
				action: dagCbor.decode(actionCbor),
				createdAt: FIXED_CREATED_AT,
				...(prevRef ? { prev: prevRef } : {}),
			};
			const cid = await computeRecordCid(record);
			const uri = `at://${did}/${ACTION}/${actionRkey(tableTid, seq)}`;
			bus.push({ did, seq, actionCbor, record });
			published.push({ seq, author: did, record, ref: { uri, cid } });
			return { uri, cid };
		},
	};
}

function buildGames(WasmAgent, dagCbor, seats) {
	const tableTid = "t";
	const dids = Array.from({ length: seats }, (_, s) => `did:plc:wasmseat${s}`);
	const tableRef = { uri: `at://${dids[0]}/re.cardco.poker.table/${tableTid}`, cid: "tablecid" };
	const tableRecord = {
		$type: "re.cardco.poker.table",
		players: dids,
		startingChips: 100,
		smallBlind: 5,
		createdAt: FIXED_CREATED_AT,
	};
	const tableCbor = dagCbor.encode(tableRecord);
	const bus = [];
	const published = []; // every published record, across all authors
	const games = dids.map((did, seat) => {
		const agent = new WasmAgent(did, new TextEncoder().encode(`wasm-seed-${seat}`));
		const publisher = realRecordPublisher(dagCbor, did, tableTid, bus, published);
		return new ChainedGame({ agent, did, tableRef, tableTid, publisher });
	});
	return { dids, tableCbor, bus, published, games };
}

async function runHand(WasmAgent, dagCbor, seats) {
	const { tableCbor, bus, published, games } = buildGames(WasmAgent, dagCbor, seats);
	for (const g of games) await g.receiveTable(tableCbor);

	let guard = 0;
	for (;;) {
		if (guard++ > 100000) throw new Error("hand did not converge");
		if (bus.length) {
			const ev = bus.shift();
			for (const g of games) {
				if (g.session.did === ev.did) continue;
				await g.deliverFirehoseAction(ev.did, ev.seq, ev.actionCbor, ev.record);
			}
			continue;
		}
		const waiting = games.find((g) => g.needsBet);
		if (waiting) {
			await waiting.submitBet(passiveBet(waiting.betOptions));
			continue;
		}
		break;
	}
	return { games, published, seats };
}

async function assertSettlementValid({ games, published, seats }) {
	for (const g of games) assert.equal(g.isComplete, true, `${g.session.did} did not complete`);

	// Every seat observed the same chain shape.
	const ref = games[0].session.chain;
	assert.ok(ref.length > 0, "empty chain");
	for (const g of games) {
		assert.deepEqual(
			g.session.chain.map((c) => ({ seq: c.seq, seat: c.seat, kind: c.kind })),
			ref.map((c) => ({ seq: c.seq, seat: c.seat, kind: c.kind })),
			`${g.session.did} observed a different chain`,
		);
	}

	// Contiguous global seq from 0; opens with commits; ends with seed reveals.
	assert.deepEqual(
		ref.map((c) => c.seq),
		ref.map((_, i) => i),
	);
	assert.equal(ref[0].kind, "commitSeed");
	const tail = ref.slice(-seats);
	assert.ok(tail.every((c) => c.kind === "verifySeed"), "chain must end with seed reveals");
	assert.equal(new Set(tail.map((c) => c.seat)).size, seats, "each seat reveals once");

	// Exactly one published record per global slot, and every record's `prev`
	// is the REAL content-addressed strong ref of the previous slot's record —
	// a genuine cross-author, contiguous, verifiable chain.
	const bySeq = new Map();
	for (const p of published) {
		assert.ok(!bySeq.has(p.seq), `duplicate publish at seq ${p.seq}`);
		bySeq.set(p.seq, p);
	}
	for (let seq = 0; seq < ref.length; seq++) {
		const rec = bySeq.get(seq);
		assert.ok(rec, `missing published record at seq ${seq}`);
		assert.equal(rec.record.seq, seq, "record seq matches global slot");
		if (seq === 0) {
			assert.equal(rec.record.prev, undefined, "first action has no prev");
		} else {
			const prevRec = bySeq.get(seq - 1);
			assert.deepEqual(
				rec.record.prev,
				prevRec.ref,
				`prev at seq ${seq} must be the previous record's strong ref`,
			);
			// The stored prev CID really is the previous record's content CID.
			assert.equal(rec.record.prev.cid, await computeRecordCid(prevRec.record));
		}
	}

	// The tip each game reports is the final action's strong ref (settle target).
	const finalRec = bySeq.get(ref.length - 1);
	for (const g of games) {
		assert.deepEqual(g.tipRef ? { cid: g.tipRef.cid } : null, { cid: finalRec.ref.cid });
	}
}

test("real WASM: two seats build one content-addressed settlement-valid chain", async (t) => {
	const { WasmAgent, dagCbor, skip } = await loadDeps();
	if (skip) return t.skip(`pkg-node/@ipld/dag-cbor unavailable: ${skip}`);
	await assertSettlementValid(await runHand(WasmAgent, dagCbor, 2));
});

test("real WASM: three seats build one content-addressed settlement-valid chain", async (t) => {
	const { WasmAgent, dagCbor, skip } = await loadDeps();
	if (skip) return t.skip(`pkg-node/@ipld/dag-cbor unavailable: ${skip}`);
	await assertSettlementValid(await runHand(WasmAgent, dagCbor, 3));
});
