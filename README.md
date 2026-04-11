# Cardcore Poker

Provably fair Texas Hold'em on the [AT Protocol](https://atproto.com), using mental poker cryptography. No trusted third party needed — the deck is shuffled and dealt through a cryptographic protocol where no single player can cheat.

## How it works

The game uses **Ristretto255 commutative encryption** so players can lock and unlock cards in any order. A two-phase shuffle+lock protocol ensures every card is encrypted by every player before positions are fixed:

1. **Shuffle phase** — each player encrypts all 52 cards with their secret key and randomly permutes the deck
2. **Lock phase** — each player swaps their shuffle key for per-position lock keys (positions are now fixed)
3. **Dealing** — players reveal their per-position lock scalars, verifiable by everyone
4. **Verification** — after the hand, players reveal their seeds and the entire game can be replayed deterministically

All randomness derives from each player's secret seed via a deterministic PRNG (ChaCha20 seeded from BLAKE2b). Seeds are committed (hashed) before the hand and revealed after for full replay verification.

## AT Protocol integration

Game actions are AT Protocol records under two collections:

- `re.cardco.poker.table` — establishes a game with player DIDs, chip counts, and blinds
- `re.cardco.poker.action` — every game action (commit, shuffle, lock, deal, bet, reveal) as a `$type`-tagged union

Actions are keyed as `{table-tid}-{seq}` for deterministic lookup and chain backward via `prev` strongRefs. Serialization uses DAG-CBOR via [dasl](https://github.com/n0-computer/dasl).

## Project structure

```
src/
├── agent.rs      # CBOR-in/CBOR-out player agent (the main interface)
├── card.rs       # Card, Rank, Suit types
├── crypto.rs     # Ristretto255 encryption (curve25519-dalek, pure Rust)
├── eval.rs       # Poker hand evaluation
├── game.rs       # Hold'em game state and betting logic
├── protocol.rs   # Protocol state machine
├── wasm.rs       # wasm-bindgen API for browser
├── lexicon/      # Generated AT Protocol types (Jacquard)
└── bin/poker.rs  # Text-based CLI
lexicons/         # AT Protocol lexicon schemas (JSON)
```

## Building

```sh
# Native
cargo build --release

# WASM (produces pkg/ with JS/TS bindings)
wasm-pack build --target web --release

# Or use just
just build-wasm
```

## Testing

```sh
# Native tests (crypto, protocol, hand eval, agent, CBOR roundtrips, fuzz)
cargo test --release

# WASM browser tests (headless Chrome)
wasm-pack test --headless --chrome --release

# All tests
just test-all

# Docker (includes Chrome + chromedriver)
just docker-build
just docker-test
```

## Playing

```sh
cargo run --bin poker
```

Hot-seat multiplayer at the terminal. The full cryptographic protocol runs — cards are actually encrypted and dealt via Ristretto255.

## WASM API

```typescript
import init, { WasmAgent } from './pkg/cardcore_poker.js';
await init();

const agent = new WasmAgent("did:plc:alice", seed);
const out = agent.receive_table(tableCbor);     // → WasmOutput
const out2 = agent.receive_action(actionCbor);   // → WasmOutput

// out.kind: "actions" | "need_bet" | "waiting"
// out.action(0): Uint8Array (DAG-CBOR to publish)

agent.bet("call");
agent.hole_cards();       // → '["Jh", "Kc"]'
agent.community_cards();  // → '["8s", "9s", "Jc", "Qd", "7s"]'
```

## License

MIT
