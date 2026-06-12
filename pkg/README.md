# Cardcore Poker

Provably fair Texas Hold'em — and Blackjack — on the [AT Protocol](https://atproto.com), using mental poker
cryptography. No trusted third party needed — the deck is shuffled and dealt through a cryptographic protocol where no
single player can cheat.

## How it works

The game uses **Ristretto255 commutative encryption** so players can lock and unlock cards in any order. A two-phase shuffle+lock protocol ensures every card is encrypted by every player before positions are fixed:

1. **Shuffle phase** — each player encrypts all 52 cards with their secret key and randomly permutes the deck
2. **Lock phase** — each player swaps their shuffle key for per-position lock keys (positions are now fixed)
3. **Dealing** — players reveal their per-position lock scalars, verifiable by everyone
4. **Verification** — after the hand, players reveal their seeds and the entire game can be replayed deterministically

All randomness derives from each player's secret seed via a deterministic PRNG (ChaCha20 seeded from BLAKE2b). Seeds are committed (hashed) before the hand and revealed after for full replay verification.

The crypto engine (`src/engine.rs`) is game-agnostic; poker and blackjack are two state machines built on the same
commit → shuffle → lock → reveal mechanics.

## Games

**Texas Hold'em** — the original. Hole cards are dealt privately (the recipient's lock key stays on the card), community
cards publicly; betting with blinds, side pots, and showdown hand evaluation.

**Blackjack** — European no-hole-card (ENHC) with a rotating banker:

- Each round one seat banks (rotating like a button); everyone else wagers against the banker's stack.
- Every card is dealt face-up via an all-players reveal — no hidden state, no peek trust problem.
- Bettors act in seat order: hit, stand, double down, split (one split max; split aces get one card), late surrender,
  and insurance when the banker shows an ace.
- The banker's hand is auto-played by fixed rules (draw to 17, stand on soft 17), so nobody — including the banker — can
  cheat.
- Standard ENHC settlement: a banker two-card 21 takes doubles and splits but pushes against a bettor blackjack;
  blackjack pays 3:2 (floored); settlement is collect-then-pay, capped by the banker's stack.

## AT Protocol integration

Game actions are AT Protocol records, one pair of collections per game:

- `re.cardco.poker.table` — establishes a poker game with player DIDs, chip counts, and blinds
- `re.cardco.poker.action` — every poker action (commit, shuffle, lock, deal, bet, reveal) as a `$type`-tagged union
- `re.cardco.blackjack.table` — establishes a blackjack game (players, starting chips, min bet)
- `re.cardco.blackjack.action` — every blackjack action (commit, shuffle, lock, deal, wager, insurance, decision,
  verify) as a `$type`-tagged union

Actions are keyed as `{table-tid}-{seq}` for deterministic lookup and chain backward via `prev` strongRefs. Serialization uses DAG-CBOR via [dasl](https://github.com/n0-computer/dasl).

## Project structure

```
src/
├── agent.rs      # CBOR-in/CBOR-out poker agent (the main interface)
├── agent_util.rs # Crypto-response builders shared by the agents
├── blackjack/    # Blackjack: eval, game rules, protocol, agent
├── card.rs       # Card, Rank, Suit types
├── crypto.rs     # Ristretto255 encryption (curve25519-dalek, pure Rust)
├── engine.rs     # Game-agnostic mental-card engine (CryptoRound)
├── eval.rs       # Poker hand evaluation
├── game.rs       # Hold'em game state and betting logic
├── protocol.rs   # Poker protocol state machine
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
import init, { WasmAgent, WasmBlackjackAgent } from "./pkg/cardcore_poker.js";
await init();

// Poker
const agent = new WasmAgent("did:plc:alice", seed);
const out = agent.receive_table(tableCbor); // → WasmOutput
const out2 = agent.receive_action(actionCbor); // → WasmOutput

// out.kind: "actions" | "need_bet" | "waiting"
// out.action(0): Uint8Array (DAG-CBOR to publish)

agent.bet("call");
agent.hole_cards(); // → '["Jh", "Kc"]'
agent.community_cards(); // → '["8s", "9s", "Jc", "Qd", "7s"]'

// Blackjack
const bj = new WasmBlackjackAgent("did:plc:alice", seed);
bj.receive_table(tableCbor); // → WasmBjOutput
bj.receive_action(actionCbor); // → WasmBjOutput

// out.kind: "actions" | "need_wager" | "need_insurance" | "need_decision" | "waiting"
// out.options: '{"min":10,"max":1000}' or '["hit","stand","double"]'

bj.act("wager:25"); // also: "insurance:yes"|"insurance:no"|"hit"|"stand"|"double"|"split"|"surrender"
bj.my_hands(); // → '[["8c","8d"]]' (two arrays after a split)
bj.banker_cards(); // → '["As","7c"]'
bj.last_round_result(); // → per-hand outcomes & payouts JSON
```

## License

MIT
