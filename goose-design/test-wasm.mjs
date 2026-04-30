import init, { WasmAgent } from '../pkg/cardcore_poker.js';
import * as dagCbor from '../node_modules/@ipld/dag-cbor/src/index.js';
await init();

const seed1 = new Uint8Array(32); crypto.getRandomValues(seed1);
const seed2 = new Uint8Array(32); crypto.getRandomValues(seed2);

const a1 = new WasmAgent("did:plc:alice", seed1);
const a2 = new WasmAgent("did:plc:bob", seed2);

const tableCbor = dagCbor.encode({
  $type: 're.cardco.poker.table',
  players: ['did:plc:alice', 'did:plc:bob'],
  startingChips: 1000,
  smallBlind: 10,
  createdAt: new Date().toISOString(),
});

console.log("=== Feed tables ===");
let o1 = a1.receive_table(tableCbor);
console.log("A1:", o1.kind, "n:", o1.action_count);
let o2 = a2.receive_table(tableCbor);
console.log("A2:", o2.kind, "n:", o2.action_count);

// Ping-pong actions between agents
for (let r = 0; r < 15; r++) {
  let changed = false;
  if (o1.kind === 'actions') {
    for (let i = 0; i < o1.action_count; i++) {
      o2 = a2.receive_action(new Uint8Array(o1.action(i)));
      changed = true;
    }
  }
  if (o2.kind === 'actions') {
    for (let i = 0; i < o2.action_count; i++) {
      o1 = a1.receive_action(new Uint8Array(o2.action(i)));
      changed = true;
    }
  }
  console.log(`R${r}: A1=${o1.kind}:${o1.action_count} A2=${o2.kind}:${o2.action_count} cards1=${a1.hole_cards()} cards2=${a2.hole_cards()} comm=${a1.community_cards()}`);
  if (!changed && o1.kind === 'waiting' && o2.kind === 'waiting') break;
}

console.log("\n=== Final ===");
console.log("A1 hole:", a1.hole_cards());
console.log("A1 comm:", a1.community_cards());
console.log("A2 hole:", a2.hole_cards());
console.log("A2 comm:", a2.community_cards());

// Check bets
o1 = a1.check_status();
o2 = a2.check_status();
console.log("A1 bet:", o1.kind, o1.bet_options);
console.log("A2 bet:", o2.kind, o2.bet_options);
