# atbloons wallet handoff — Cardcore client

This directory is the Cardcore side of the protocol-v3 atbloons wallet handoff. It lets a poker seat escrow a buy-in in atbloons, play the hand in chips, and settle the chip stacks back to atbloons. The paid flow is optional. When no wallet is configured, the game runs its normal unpaid hands and none of this code runs.

atbloons is a resettable AT Protocol currency testnet. It is not a blockchain or mainnet. Do not treat testnet outputs as real money.

## Trust boundary

The client sends only public game references to the wallet and reads only a public receipt. The confidential wallet holds every OAuth token and DPoP key and re-verifies all repository evidence through its node before it spends. The client never receives a secret.

## Modules

| File | Responsibility |
| --- | --- |
| `handoff.js` | Build and encode intents, build the wallet URL, parse and validate receipts. No dependency. |
| `paid-hand.js` | `PaidHandController` — drive one seat through one hand across full-page redirects. |
| `config.js` | Resolve the wallet URL and network tuple; discover the tuple from the managed-wallet node. |
| `contract-lookup.js` | Find the contract strong reference for a table from the host repo, so a seat funds without a paste. |
| `chained-driver.js` | `ChainedSession` — drive one seat through a paid hand as one globally ordered, settlement-valid action chain. |
| `chained-game.js` | Live wiring: `ChainedGame`, content-addressed `actionStrongRef`, and the publisher-to-driver `publish` wrapper. |
| `handoff.test.js` | Offline unit tests for the wire contract. |
| `paid-hand.test.js` | Offline lifecycle tests for the controller. |
| `config.test.js` | Offline tests for configuration and node discovery. |
| `contract-lookup.test.js` | Offline tests for contract discovery. |
| `chained-driver.test.js` | Offline tests for the paid-hand driver (order, links, buffering, seed tail). |
| `chained-game.test.js` | Offline tests for the strong-ref and publish helpers. |
| `chained-wasm.test.js` | Real-engine proof: seats build one content-addressed settlement-valid chain. |
| `interop.test.js` | Cross-repository wire-compatibility proof against the atbloons Kotlin test. |

The Svelte panel is `packages/web/src/components/AtbloonsHandoff.svelte`. `GameRoom.svelte` mounts it for a seated player when a wallet is configured, and it drives the hand through `ChainedGame` in paid mode.

## Configuration

The wallet is the atbloons node's managed wallet, so the wallet origin is also the node origin. A deployment needs to set only the wallet URL:

- `VITE_ATBLOONS_WALLET_URL` — the managed-wallet public origin (HTTPS, or exact loopback HTTP for local development).

The client then discovers the exact network tuple from the node's public `GET /v1/network` descriptor (`discoverAtbloonsConfig`) and caches it per browser. A player may instead store a JSON override under the `atbloons.config` key in `localStorage`.

A deployment that does not want a discovery request can pin the tuple by hand with these optional variables:

- `VITE_ATBLOONS_NETWORK_ID` — the atbloons network id.
- `VITE_ATBLOONS_GENESIS_HASH` — the 64-character genesis hash.
- `VITE_ATBLOONS_PROTOCOL_VERSION` — optional; defaults to `v3`.

When both `VITE_ATBLOONS_NETWORK_ID` and `VITE_ATBLOONS_GENESIS_HASH` are set, the client uses the pinned tuple and makes no discovery request. The tuple must match the operator's atbloons node exactly. A different genesis hash is a different network.

## Hand lifecycle

1. The seat-0 host finalizes the `re.cardco.poker.table` record, then proposes the contract and funds its own seat.
2. Each other seat funds its seat. The client discovers the contract from the host repo (`contract-lookup.js`), so a seat funds in one click. Manual paste stays as a fallback. A funded seat may withdraw before activation.
3. The host activates after every funding confirms.
4. The players play the hand in chips. A paid hand publishes one settlement-valid chain (see below).
5. The host settles. The panel uses the chain tip as the terminal action, so settlement is one click. The wallet node replays the chain and pays each stack times `soulsPerChip`.

The wallet derives all money terms from verified state. The client cannot set a payout.

## Settlement-valid action chain

A paid hand publishes one bounded, contiguous, globally ordered chain of `re.cardco.poker.action` records. The atbloons `CARDCORE_POKER_V1` evaluator accepts this shape.

The engine drives the chain in chained mode:

- `WasmAgent.set_chained(true)` stops the agent from auto-emitting.
- `next_action()` reports who acts next in the canonical global order.
- `produce_next()` and `produce_bet()` build a record without applying it.
- The driver publishes the record with the running global `prev` and `seq`, then feeds it back through `receive_action`.

Each record's `prev` is the content-addressed strong reference of the previous action, often in another player's repository. The chain ends with one `verifySeed` per seat — the mandatory seed reveal. `ChainedSession` drives one seat; `ChainedGame` wires it to the publisher and the firehose.

## Run the tests

```bash
node --test packages/web/src/lib/atbloons/*.test.js
```

These tests run offline and need no browser and no external package.

## Status

The paid path is now wired end to end in the client. `GameRoom.svelte` selects the engine at mount. When the managed wallet is configured, a seat plays the hand through `ChainedGame` as one settlement-valid chain. With no wallet, it plays through the unpaid `PlayerSession`, unchanged. The firehose gives the decoded action record to `deliverFirehoseAction`, and the completed chain tip becomes the settle terminal action, so settlement stays one click.

The chained engine mode, the WASM bindings, the `ChainedSession` driver, and the `ChainedGame` wiring are complete and tested. Native Rust tests (`tests/chained_transcript.rs`) prove that separate agents build one contiguous chain that ends with every seat's seed reveal. The real-engine test `chained-wasm.test.js` proves the same chain with real content-addressed `prev` links. The driver also replays our own published actions on a reload, so a resumed paid hand does not ask again for a bet we already published.

Only external validation remains. A full browser end-to-end run needs a live PDS development environment and a browser (Playwright). The client change is inert when no wallet is configured, so unpaid play stays the same. The clean production build and the offline suites are the offline evidence.
