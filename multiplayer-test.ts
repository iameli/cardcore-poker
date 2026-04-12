/**
 * Multiplayer test: 4 players over a real AT Protocol PDS.
 *
 * Usage: PDS_URL=http://localhost:36607 npx tsx multiplayer-test.ts
 */

import { AtpAgent } from "@atproto/api";
import WebSocket from "ws";
import { decode as cborDecode, encode as cborEncode, decodeMultiple } from "cbor-x";

const wasmModule = require("./pkg-node/cardcore_poker.js");

const PDS_URL = process.env.PDS_URL || "http://localhost:36607";
const NUM_PLAYERS = 4;
const TABLE_RKEY = `t${Date.now() % 100000}`;

interface Player {
  handle: string;
  did: string;
  agent: AtpAgent;
  wasmAgent: any;
}

// Global action CBOR store: "did:rkey" → original CBOR
const actionStore = new Map<string, Uint8Array>();
// Global seq tracker per player DID
const seqTracker = new Map<string, number>();
// Global ordered action log: seq → { did, rkey, cbor }
const actionLog: { did: string; rkey: string; cbor: Uint8Array }[] = [];

async function main() {
  console.log(`Multiplayer: ${NUM_PLAYERS} players on ${PDS_URL}\n`);

  // Register accounts
  const players: Player[] = [];
  for (let i = 0; i < NUM_PLAYERS; i++) {
    const handle = `p${i}-${Date.now() % 100000}.test`;
    const password = `pass${Date.now()}`;
    const agent = new AtpAgent({ service: PDS_URL });
    await agent.createAccount({
      handle, password,
      email: `${handle.replace('.test','')}@test.invalid`,
    });
    const did = agent.session!.did;
    const seed = new TextEncoder().encode(`mpseed_${i}_${Date.now()}`);
    const wasmAgent = new wasmModule.WasmAgent(did, seed);
    players.push({ handle, did, agent, wasmAgent });
    seqTracker.set(did, 0);
    console.log(`  Player ${i}: ${did}`);
  }

  // Table consensus
  const tableRecord = {
    $type: "re.cardco.poker.table",
    players: players.map(p => p.did),
    startingChips: 1000,
    smallBlind: 10,
    createdAt: new Date().toISOString(),
  };
  for (const p of players) {
    await p.agent.api.com.atproto.repo.putRecord({
      repo: p.did, collection: "re.cardco.poker.table",
      rkey: TABLE_RKEY, record: tableRecord,
    });
  }
  console.log(`\nTable created: ${TABLE_RKEY}`);

  const tableCbor = cborEncode(tableRecord);

  // Subscribe to firehose FIRST, then write initial actions
  await new Promise<void>((resolve, reject) => {
    const timeout = setTimeout(() => { ws.close(); reject(new Error("timeout")); }, 120000);
    const wsUrl = PDS_URL.replace("http", "ws") + "/xrpc/com.atproto.sync.subscribeRepos";
    const ws = new WebSocket(wsUrl);
    const playerDids = new Set(players.map(p => p.did));
    const seen = new Set<string>();
    let done = false;
    let started = false;

    let queue = Promise.resolve();

    ws.on("open", () => {
      console.log("Firehose connected\n");
      // Now write commitSeeds — firehose will pick them up
      queue = queue.then(async () => {
        for (const p of players) {
          const out = p.wasmAgent.receive_table(tableCbor);
          await writeActions(p, out, players[0].did);
        }
        console.log("CommitSeeds written\n");
        started = true;
      });
    });

    ws.on("message", (data: Buffer) => {
      if (done) return;
      queue = queue.then(async () => {
        if (done) return;
        try {
          const frame = decodeFrame(data);
          if (!frame) { return; }
          if (frame.t !== "#commit") return;
          if (!playerDids.has(frame.repo)) return;

          for (const op of frame.ops || []) {
            if (!op.path.startsWith("re.cardco.poker.action/")) continue;
            const rkey = op.path.split("/")[1];
            const key = `${frame.repo}:${rkey}`;
            if (seen.has(key)) continue;
            seen.add(key);

            const cbor = actionStore.get(key);
            if (!cbor) continue;

            const decoded = cborDecode(Buffer.from(cbor));
            const type = decoded?.$type?.split("#")[1] || "?";
            console.log(`  ${frame.repo.slice(-8)} → ${type}`);

            // Feed to all OTHER players
            for (const p of players) {
              if (p.did === frame.repo) continue;
              try {
                let out = p.wasmAgent.receive_action(new Uint8Array(cbor));
                await writeActions(p, out, players[0].did);

                // Drain auto-responses and bets
                for (let i = 0; i < 50; i++) {
                  out = p.wasmAgent.check_status();
                  if (out.kind === "actions" && out.action_count > 0) {
                    await writeActions(p, out, players[0].did);
                  } else if (out.kind === "need_bet") {
                    const bet = pickBet(JSON.parse(out.bet_options));
                    out = p.wasmAgent.bet(bet);
                    await writeActions(p, out, players[0].did);
                  } else break;
                }
              } catch (e: any) {
                // Out of order or duplicate — ignore
              }
            }

            // Check for completion
            if (type === "verifySeed") {
              const verifyCount = Array.from(seen).filter(k => {
                const c = actionStore.get(k.split(":").slice(0).join(":").replace(/^[^:]+:/, (m) => m));
                if (!c) return false;
                try { return cborDecode(Buffer.from(c))?.$type?.includes("verifySeed"); } catch { return false; }
              }).length;
              // Simple: if we've seen a verifySeed, wait a bit then check
              await new Promise(r => setTimeout(r, 1000));
              const allVerifyActions = Array.from(actionStore.entries())
                .filter(([_, v]) => { try { return cborDecode(Buffer.from(v))?.$type?.includes("verifySeed"); } catch { return false; } });
              if (allVerifyActions.length >= NUM_PLAYERS) {
                done = true;
                clearTimeout(timeout);
                ws.close();
                resolve();
                return;
              }
            }
          }
        } catch {}
      });
    });

    ws.on("error", e => { clearTimeout(timeout); reject(e); });
    ws.on("open", () => console.log("Firehose connected\n"));
  });

  // Results
  console.log("\n=== RESULTS ===");
  for (const p of players) {
    console.log(`  ${p.handle}: ${p.wasmAgent.hole_cards()}`);
  }
  console.log(`  Community: ${players[0].wasmAgent.community_cards()}`);
  console.log(`  Actions: ${actionStore.size} records written to PDS`);

  for (const p of players) p.wasmAgent.free();
  console.log("\nMultiplayer test PASSED!");
  process.exit(0);
}

async function writeActions(player: Player, output: any, tableDid: string) {
  for (let i = 0; i < output.action_count; i++) {
    const cbor = output.action(i);
    const seq = seqTracker.get(player.did) || 0;
    const rkey = `${TABLE_RKEY}-${seq}`;
    const decoded = cborDecode(Buffer.from(cbor));

    actionStore.set(`${player.did}:${rkey}`, cbor);
    actionLog.push({ did: player.did, rkey, cbor });

    const record: any = {
      $type: "re.cardco.poker.action",
      table: { uri: `at://${tableDid}/re.cardco.poker.table/${TABLE_RKEY}`, cid: "bafyplaceholder" },
      seq, action: decoded,
      createdAt: new Date().toISOString(),
    };

    await player.agent.api.com.atproto.repo.putRecord({
      repo: player.did, collection: "re.cardco.poker.action",
      rkey, record,
    });
    seqTracker.set(player.did, seq + 1);
  }
}

function decodeFrame(data: Buffer): any | null {
  try {
    const values: any[] = [];
    decodeMultiple(data, (v: any) => values.push(v));
    if (values.length < 2) return null;
    const [header, body] = values;
    if (header?.op !== 1) return null;
    return { t: header.t, ...body };
  } catch { return null; }
}

let betRng = 12345;
function pickBet(options: any[]): string {
  betRng = Math.imul(betRng, 1103515245) + 12345;
  const roll = Math.abs(betRng % 100);
  const hasCheck = options.some((o: any) => o === "Check" || o?.Check !== undefined);
  if (roll < 50) return hasCheck ? "check" : "call";
  if (roll < 80) return "call";
  if (roll < 95) return "allIn";
  return hasCheck ? "check" : "fold";
}

main().catch(e => { console.error("Fatal:", e.message || e); process.exit(1); });
