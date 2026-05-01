# @cardcore/web

Texas Hold'em with mental poker privacy — no trusted dealer, no relay server.
Players encrypt, shuffle, and deal cards collaboratively using Ristretto255
commutative encryption (the Rust crate at the repo root, compiled to WASM).
All multiplayer transport runs over each player's own AT Protocol PDS.

## Architecture

```
                 each player's browser (Svelte 5)
                 ┌──────────────────────────┐
                 │  Lobby / GameRoom        │
                 │  PokerTable / Card / …   │
                 │                          │
                 │  PlayerSession ──────┐   │
                 │  WasmAgent           │   │
                 │  (Ristretto255)      │   │
                 │                      ▼   │
                 │  Publisher  ┐  FirehoseSubscriber
                 │             │        │   │
                 └─────────────┼────────┼───┘
                  putRecord    │        │  subscribeRepos
                               ▼        ▲
                          ┌──── AT Protocol PDS ────┐
                          │  re.cardco.poker.table  │
                          │  re.cardco.poker.action │
                          └─────────────────────────┘
```

No back-end of our own. The WasmAgent runs the cryptographic protocol
in-browser, the Publisher writes the resulting actions to the user's PDS
via `com.atproto.repo.putRecord`, and the FirehoseSubscriber listens to
each peer's PDS via `com.atproto.sync.subscribeRepos` to feed peer
actions back into the local agent.

## Modules

| Path                             | Purpose                                                                                        |
| -------------------------------- | ---------------------------------------------------------------------------------------------- |
| `src/App.svelte`                 | Page routing (signin → lobby → game)                                                           |
| `src/components/SignIn.svelte`   | OAuth handle entry + "Play in Demo Mode"                                                       |
| `src/components/Lobby.svelte`    | Create a table (resolve opponent's handle, publish table record) or paste an AT URI to join    |
| `src/components/GameRoom.svelte` | Loads the table at the AT URI, drives the local agent, renders cards/bets                      |
| `src/lib/atproto.js`             | OAuth via `@atcute/oauth-browser-client`, DID/handle/PDS resolvers                             |
| `src/lib/demo-pds.js`            | Demo signin (auto-`createAccount` on the local PDS in dev)                                     |
| `src/lib/cardcore-wasm.js`       | Bridge to the Rust WASM crate                                                                  |
| `src/lib/game-session.js`        | `PlayerSession` — wraps a WasmAgent with a publish callback + observable state                 |
| `src/lib/transport.js`           | `Publisher` — `createRecord`/`putRecord` for table + action lexicons                           |
| `src/lib/firehose.js`            | `FirehoseSubscriber` — backfill via `listRecords`, live via `subscribeRepos`, CAR-block decode |
| `src/lib/poker-engine.js`        | Hand evaluation (kept for showdown UI, not for protocol)                                       |
| `src/lib/atproto-publisher.js`   | Lexicon constants + record builders                                                            |

## Run

Local dev boots a real `@atproto/pds` (in `packages/dev-env/`) on
`:2583` and Vite on `:5173`. Vite proxies `/xrpc` (HTTP and WebSocket
upgrade) to the PDS, so all AT Protocol calls share the dev-server origin.

```bash
pnpm dev:all          # PDS + Vite together
# or:
pnpm dev:pds          # just the PDS
pnpm dev              # just Vite
```

Open `http://localhost:5173`. "Play in Demo Mode" auto-creates a
`demo-{slug}.test` account on the local PDS and gives you a session.
For a second player, open another browser (or incognito) — separate
localStorage = separate demo identity. Player A enters Player B's handle,
clicks Create Table, copies the AT URI from the GameRoom header. Player B
pastes it into "Join Existing." The cryptographic deal runs over the PDS.

In production (`pnpm build`) the OAuth client_id from
`public/oauth-client-metadata.json` is used; users sign in with their
real Bluesky handle. The static bundle in `dist/` deploys anywhere.

## Configuration

Environment variables read at build/dev time by `vite.config.js`:

- `PDS_PORT` — default `2583`
- `VITE_PORT` — default `5173`
- `VITE_HOST` — extra allowed dev-server host
- `SLINGSHOT_URL` — identity resolver. Default in prod:
  `https://slingshot.microcosm.blue`. In dev, empty (handle/DID lookups
  hit the local PDS via the `/xrpc` proxy).
- `FIREHOSE_URL` — filtered firehose endpoint that accepts `wantedDids`
  query params. Default in prod: `wss://firehose.channel`. When set, the
  client opens one WebSocket and the server filters server-side to just
  the player DIDs in this hand. When empty (dev), the client falls back
  to one socket per peer PDS.

## Tests

```bash
pnpm test          # Playwright; non-headless locally, headless in CI
pnpm test:headed   # force headed
```

The Playwright spec spins up two browser contexts on the dev server,
demo-signs-in each, has A create a table including B's handle, and B
joins via the AT URI. Both then run the real Ristretto255 deal until
one folds. CI sets `CI=true` to flip to headless.
