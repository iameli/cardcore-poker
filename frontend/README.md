# Cardcore Poker

Texas Hold'em with mental poker privacy — no trusted dealer required. Players encrypt, shuffle, and deal cards collaboratively using NaCl box encryption so no single party ever sees the full deck.

## Architecture

```
browser (Svelte 5)          server (Node.js)
┌──────────────────┐        ┌─────────────────┐
│  PokerTable      │        │  WebSocket room │
│  PlayerSeat      │  ws:// │  server on :3003 │
│  ActionBar       │◄──────►│                 │
│  GameLog         │        │  Rooms in memory │
│  Lobby           │        │  Action relay    │
│                  │        │  API: /api/rooms │
│  mental-poker.js │        └─────────────────┘
│  poker-engine.js │
│  room.js         │
└──────────────────┘
```

- **`server/index.js`** — Express + `ws` WebSocket server. Rooms live in a `Map`, messages are broadcast to all players. Seat selection, ready check, game start, and action relay.

- **`src/lib/room.js`** — Singleton WebSocket client. Buffers messages until connected, reconnects with exponential backoff.

- **`src/lib/mental-poker.js`** — NaCl box encryption via `tweetnacl`. Each player generates an ed25519 keypair, encrypts every card in rotation, and cards are decrypted one layer at a time when dealt.

- **`src/lib/poker-engine.js`** — Hand evaluation (best 5 of 7), betting rounds (preflop → river → showdown), and available action computation.

- **`src/components/`** — Svelte 5 components: `PokerTable`, `PlayerSeat`, `Card`, `ActionBar`, `GameLog`, `Lobby`, `SignIn`, `GameRoom`.

- **Spritesheets** — `public/clubs.png`, `diamonds.png`, `hearts.png`, `spades.png` — each 520×59 (13 ranks × 40×59px).

- **Auth** — AT Protocol OAuth (`@atcute`) or local demo identity stored in localStorage.

## Run

```bash
npm install
npm run dev:all        # starts server (:3003) + client (:5173)
```

Or separately:

```bash
npm run dev            # Vite dev server :5173
npm run dev:server     # WebSocket server :3003
```

Vite proxies `/ws` and `/api` to the backend so both appear on `:5173` in the browser.

## Key files

| Path | Purpose |
|---|---|
| `server/index.js` | Room server, WebSocket handler |
| `src/App.svelte` | Page routing (signin → lobby → game) |
| `src/components/GameRoom.svelte` | Waiting room + game layout |
| `src/components/PokerTable.svelte` | Table, community cards, seat positioning |
| `src/components/Card.svelte` | Single card rendering |
| `src/lib/room.js` | WebSocket client singleton |
| `src/lib/mental-poker.js` | Card encryption & shuffle |
| `src/lib/poker-engine.js` | Hand eval, betting, phases |
| `vite.config.js` | Vite config with OAuth env & proxy |
