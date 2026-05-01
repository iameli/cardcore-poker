// Node integration test for the WASM ↔ WebSocket multiplayer flow.
//
// Spins up the dev WebSocket room server in-process, then drives two players
// through the protocol using the SAME PlayerSession bridge the Svelte UI uses.
// Validates that:
//   1. Each agent decrypts its own hole cards (2 cards per player).
//   2. After the deal, exactly one player needs a bet decision.
//   3. A fold by the SB causes the game to complete.
//
// Run with:  node tests/multiplayer.mjs
//
// This is a structural smoke test — it runs the real Ristretto255 protocol
// through the real WebSocket relay, but skips the browser/Svelte rendering.

import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import WebSocket from "ws";
import { encode as cborEncode } from "@ipld/dag-cbor";

// Use the node-target WASM build directly (not the browser bundle).
const wasm = await import("../../../pkg-node/cardcore_poker.js");

const WS_URL = "ws://127.0.0.1:3003";
const HTTP_URL = "http://127.0.0.1:3003";

function uint8ToBase64(u8) {
  return Buffer.from(u8).toString("base64");
}
function base64ToUint8(b64) {
  return new Uint8Array(Buffer.from(b64, "base64"));
}

function buildTableCbor({ players, startingChips, smallBlind }) {
  return cborEncode({
    $type: "re.cardco.poker.table",
    players,
    startingChips,
    smallBlind,
    createdAt: new Date().toISOString(),
  });
}

class TestPlayer {
  constructor({ name, did, seed }) {
    this.name = name;
    this.did = did;
    this.playerId = `@${name}`;
    this.agent = new wasm.WasmAgent(did, seed);
    this.ws = null;
    this.roomId = null;
    this.gameStarted = false;
    this.holeCards = [];
    this.communityCards = [];
    this.needsBet = false;
    this.betOptions = [];
  }

  connect(roomId) {
    this.roomId = roomId;
    this.ws = new WebSocket(WS_URL);
    return new Promise((resolve, reject) => {
      this.ws.once("open", () => {
        this.ws.send(
          JSON.stringify({
            type: "join",
            playerId: this.playerId,
            roomId,
            playerName: this.name,
            did: this.did,
          }),
        );
        resolve();
      });
      this.ws.on("message", (raw) => this.onMessage(raw));
      this.ws.once("error", reject);
    });
  }

  send(msg) {
    this.ws.send(JSON.stringify(msg));
  }

  sit(seat) {
    this.send({ type: "sit", seat, playerId: this.playerId, roomId: this.roomId });
  }
  ready() {
    this.send({ type: "ready", playerId: this.playerId, roomId: this.roomId });
  }

  broadcastAction(payload) {
    this.send({ type: "action", playerId: this.playerId, roomId: this.roomId, action: payload });
  }

  feedTable(tableCbor, isDealer) {
    if (isDealer) {
      this.broadcastAction({ type: "wasm_table", cbor: uint8ToBase64(tableCbor) });
    }
    this.processOutput(this.agent.receive_table(tableCbor));
  }

  receiveTable(b64) {
    const out = this.agent.receive_table(base64ToUint8(b64));
    this.processOutput(out);
  }

  receiveAction(b64) {
    const out = this.agent.receive_action(base64ToUint8(b64));
    this.processOutput(out);
  }

  processOutput(output) {
    if (output.kind === "actions") {
      for (let i = 0; i < output.action_count; i++) {
        const cbor = new Uint8Array(output.action(i));
        this.broadcastAction({ type: "wasm_action", cbor: uint8ToBase64(cbor) });
      }
      this.refresh();
    } else if (output.kind === "need_bet") {
      this.needsBet = true;
      try {
        this.betOptions = JSON.parse(output.bet_options);
      } catch {
        this.betOptions = [];
      }
      this.refresh();
    } else {
      this.needsBet = false;
      this.refresh();
    }
  }

  refresh() {
    this.holeCards = JSON.parse(this.agent.hole_cards());
    this.communityCards = JSON.parse(this.agent.community_cards());
  }

  bet(action) {
    this.needsBet = false;
    this.processOutput(this.agent.bet(action));
  }

  onMessage(raw) {
    let msg;
    try {
      msg = JSON.parse(raw.toString());
    } catch {
      return;
    }
    if (msg.type === "game_start") {
      this.gameStarted = true;
      this.gameStartPlayers = msg.players;
    } else if (msg.type === "game_action") {
      if (msg.playerId === this.playerId) return; // skip our own
      if (msg.action.type === "wasm_table") this.receiveTable(msg.action.cbor);
      else if (msg.action.type === "wasm_action") this.receiveAction(msg.action.cbor);
    }
  }

  close() {
    try {
      this.ws?.close();
    } catch {}
  }
}

// ─── Driver ──────────────────────────────────────────────────────────

let serverProc;
async function startServer() {
  serverProc = spawn("node", ["server/index.js"], {
    cwd: new URL("..", import.meta.url),
    stdio: ["ignore", "pipe", "pipe"],
  });
  serverProc.stdout.on("data", (d) => process.stdout.write(`[srv] ${d}`));
  serverProc.stderr.on("data", (d) => process.stderr.write(`[srv-err] ${d}`));
  // Wait until /api/rooms responds
  for (let i = 0; i < 50; i++) {
    try {
      const res = await fetch(`${HTTP_URL}/api/rooms`);
      if (res.ok) return;
    } catch {}
    await sleep(100);
  }
  throw new Error("server did not start");
}

async function stopServer() {
  if (serverProc) serverProc.kill();
}

function expect(cond, msg) {
  if (!cond) throw new Error(`ASSERT: ${msg}`);
}

async function waitFor(predicate, msg, timeoutMs = 30000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (predicate()) return;
    await sleep(50);
  }
  throw new Error(`waitFor timeout: ${msg}`);
}

async function main() {
  await startServer();

  const res = await fetch(`${HTTP_URL}/api/rooms`, { method: "POST" });
  const { roomId } = await res.json();
  console.log(`Created room ${roomId}`);

  const seed = (s) => new TextEncoder().encode(`seed-${s}`.padEnd(32, "!").slice(0, 32));
  const alice = new TestPlayer({
    name: "alice",
    did: "did:plc:alicetest12345",
    seed: seed("alice"),
  });
  const bob = new TestPlayer({ name: "bob", did: "did:plc:bobtest1234567", seed: seed("bob") });

  await alice.connect(roomId);
  await bob.connect(roomId);
  await sleep(200);

  alice.sit(0);
  bob.sit(1);
  await sleep(200);

  alice.ready();
  bob.ready();

  await waitFor(() => alice.gameStarted && bob.gameStarted, "game_start");
  console.log("Game started");

  // Alice is dealer (seat 0). She broadcasts the wasm_table.
  const players = [alice, bob];
  const sortedDids = players.map((p) => p.did);
  const tableCbor = buildTableCbor({ players: sortedDids, startingChips: 1000, smallBlind: 10 });
  alice.feedTable(tableCbor, true);

  // Wait for cards + bet decision somewhere.
  await waitFor(
    () => alice.holeCards.length === 2 && bob.holeCards.length === 2,
    "hole cards dealt to both players",
  );
  console.log(`alice hole=${alice.holeCards.join(",")}  bob hole=${bob.holeCards.join(",")}`);
  expect(alice.holeCards.length === 2, "alice has 2 hole cards");
  expect(bob.holeCards.length === 2, "bob has 2 hole cards");
  expect(JSON.stringify(alice.holeCards) !== JSON.stringify(bob.holeCards), "hole cards differ");

  // Wait for one player to need a bet.
  await waitFor(() => alice.needsBet || bob.needsBet, "first bet decision");
  const acting = alice.needsBet ? alice : bob;
  console.log(`${acting.name} needs bet, options=${JSON.stringify(acting.betOptions)}`);
  expect(Array.isArray(acting.betOptions), "bet_options is JSON array");
  expect(acting.betOptions.length > 0, "at least one bet option offered");
  // We should see a Raise object in the options (since the SB or BB pre-flop)
  const hasRaise = acting.betOptions.some((o) => typeof o === "object" && "Raise" in o);
  expect(hasRaise, "a Raise option is offered");

  // Acting player folds → game completes (other player wins).
  acting.bet("fold");

  await waitFor(
    () => alice.agent.phase() === "Complete" && bob.agent.phase() === "Complete",
    "both reach Complete phase after fold",
  );
  console.log(
    `Both reached Complete; alice phase=${alice.agent.phase()}, bob phase=${bob.agent.phase()}`,
  );

  alice.close();
  bob.close();
  console.log("\n✅ All checks passed.");
}

const exitCode = await main()
  .then(() => 0)
  .catch((e) => {
    console.error("\n❌", e);
    return 1;
  });
await stopServer();
process.exit(exitCode);
