<script>
  import { onMount } from "svelte";
  import PokerTable from "./PokerTable.svelte";
  import ActionBar from "./ActionBar.svelte";
  import GameLog from "./GameLog.svelte";
  import { roomClient } from "../lib/room.js";
  import { PlayerSession, buildTableCbor, generateSeed } from "../lib/game-session.js";
  import { initWasm } from "../lib/cardcore-wasm.js";
  import { ATPublisher } from "../lib/atproto-publisher.js";
  import { fetchProfile, resolveHandles } from "../lib/atproto.js";
  import { evaluateHand, compareHands, GAME_PHASES } from "../lib/poker-engine.js";

  let { session, roomId, spectating = false, onLeaveRoom } = $props();

  // ─── Local State ──────────────────────────────────────────────────
  let playerId = $state("");
  let playerName = $state("");
  let room = $state({ players: [], id: "" });
  let gameState = $state(null);
  let logEvents = $state([]);
  let connected = $state(false);
  let error = $state("");
  let ourSeat = $state(-1);

  // WASM session
  let wasmSession = null;
  let _seed = null;
  let _hadCards = false;
  let _restarting = false;
  let _handOver = false;
  let decryptedHoleCards = $state({});
  let decryptedCommunityCards = $state([]);
  let availableActions = $state([]);
  let raiseContext = $state(null);
  let isOurTurn = $state(false);

  // Avatar & handle resolution
  let userAvatar = $state(null);
  let handleMap = $state({});

  // ─── Derived ──────────────────────────────────────────────────────
  const ourPlayerId = $derived(playerId);
  const gamePhase = $derived(gameState?.phase || "idle");
  const playerMap = $derived(gameState?.players || {});
  const playerOrder = $derived(gameState?.playerOrder || []);
  const playerDids = $derived.by(() => {
    const map = {};
    for (const p of room?.players || []) {
      if (p.did) map[p.id] = p.did;
    }
    for (const p of Object.values(gameState?.players || {})) {
      if (p.did) map[p.id] = p.did;
    }
    return map;
  });

  // ─── Initialize ───────────────────────────────────────────────────
  let _pid = null;
  function ensurePlayerId() {
    if (_pid) return _pid;
    _pid = session?.did
      ? `@${session.did}`
      : `@${session?.handle || session?.name || "Player"}-${Math.random().toString(36).slice(2, 8)}`;
    return _pid;
  }
  let _wired = false;

  onMount(async () => {
    await initWasm();
    const savedSeed = localStorage.getItem("cardcore_seed_" + roomId);
    if (savedSeed) _seed = new Uint8Array(savedSeed.split(",").map(Number));

    const pid = ensurePlayerId();
    const pName = session?.handle || session?.name || "Player";
    playerName = pName;
    playerId = pid;

    // Pass DID for both real OAuth and demo identities — the WASM lexicon
    // requires valid DIDs in the table record.
    const ownDid = session?.did || null;
    addLog("Connecting to room...");
    roomClient.connect(pid, roomId, pName, ownDid);

    // Fetch own avatar (real OAuth sessions only — demo DIDs aren't resolvable)
    if (session?.session && ownDid) {
      fetchProfile(ownDid).then((profile) => {
        if (profile?.avatar) userAvatar = profile.avatar;
      });
    }

    // Resolve handles for all players when room updates (real sessions only)
    const resolveAllHandles = async () => {
      if (!session?.session) return;
      const dids = (room?.players || []).map((p) => p.did).filter(Boolean);
      if (dids.length === 0) return;
      const map = await resolveHandles(dids);
      handleMap = Object.fromEntries(map);
    };

    if (!_wired) {
      _wired = true;

      roomClient.on("connected", () => {
        connected = true;
        addLog("Connected to room!");
      });

      roomClient.on("disconnected", () => {
        connected = false;
        addLog("Disconnected. Reconnecting...");
      });

      roomClient.on("room_update", (data) => {
        room = data.room;
        const our = data.room.players.find((p) => p.id === playerId);
        if (our) ourSeat = our.seat;
        resolveAllHandles();
      });

      roomClient.on("game_start", (data) => {
        addLog("Game starting!");
        initWasmGame(data.players);
      });

      roomClient.on("game_state_sync", (data) => {
        addLog("Rejoining active game...");
        replayGameState(data.players, data.history);
      });

      roomClient.on("game_action", (data) => {
        if (data.playerId !== ourPlayerId) {
          receiveRemoteAction(data.playerId, data.action);
        }
      });
    }

    return () => {
      roomClient.destroy();
      _wired = false;
    };
  });

  // ─── Logging ──────────────────────────────────────────────────────
  function addLog(msg) {
    logEvents = [...logEvents, msg];
    if (logEvents.length > 50) logEvents = logEvents.slice(-50);
  }

  // ─── Sit / Ready ──────────────────────────────────────────────────
  function sitAtSeat(seatIndex) {
    if (ourSeat === seatIndex) return;
    ourSeat = seatIndex;
    roomClient.sit(seatIndex);
    addLog(`Sat at seat ${seatIndex + 1}`);
  }

  function readyUp() {
    roomClient.ready();
    addLog("Ready!");
  }

  // ─── WASM Game Init ───────────────────────────────────────────────
  async function initWasmGame(players) {
    if (spectating || !players || players.length < 2) return;

    // Sort by seat so positional order matches the WASM agent's seat indexing
    const sortedPlayers = players.slice().sort((a, b) => a.seat - b.seat);
    const playerIds = sortedPlayers.map((p) => p.id);
    const dids = sortedPlayers.map((p) => p.did || p.id);
    const isDealer = ourPlayerId === playerIds[0];

    // Generate our seed and create session. Reuse if we're rejoining mid-hand.
    if (_restarting || !localStorage.getItem("cardcore_seed_" + roomId)) {
      _seed = generateSeed();
    }
    localStorage.setItem("cardcore_seed_" + roomId, Array.from(_seed).join(","));
    if (wasmSession) {
      wasmSession.destroy();
      wasmSession = null;
    }
    _hadCards = false;
    _handOver = false;
    decryptedHoleCards = {};
    decryptedCommunityCards = [];

    const ourPlayer = sortedPlayers.find((p) => p.id === ourPlayerId);
    const did = ourPlayer?.did || session?.did || ourPlayerId;

    wasmSession = new PlayerSession({
      did,
      seed: _seed,
      send: (cbor) => {
        roomClient.sendAction({
          type: "wasm_action",
          cbor: uint8ToBase64(cbor),
        });
      },
    });

    // Build synthetic gameState. seat indices line up with sortedPlayers order.
    const playersObj = {};
    for (let i = 0; i < sortedPlayers.length; i++) {
      const p = sortedPlayers[i];
      playersObj[p.id] = {
        id: p.id,
        name: p.name,
        did: p.did || null,
        chips: 1000,
        bet: 0,
        folded: false,
        seat: i,
      };
    }

    gameState = {
      players: playersObj,
      playerOrder: playerIds,
      phase: GAME_PHASES.PREFLOP,
      pot: 0,
      currentBet: 2,
      bigBlind: 2,
      currentPlayer: playerIds[0],
      actedThisRound: [],
    };

    if (isDealer) {
      addLog("Dealer: initializing WASM table...");
      const tableCbor = buildTableCbor({
        players: dids,
        startingChips: 1000,
        smallBlind: 10,
      });
      roomClient.sendAction({
        type: "wasm_table",
        cbor: uint8ToBase64(tableCbor),
      });
      if (session?.session?.fetchHandler) {
        try {
          const publisher = new ATPublisher({
            handler: session.session.fetchHandler,
            did: session.did,
          });
          const result = await publisher.createTable({
            players: dids,
            startingChips: 1000,
            smallBlind: 10,
          });
          addLog("Table saved to AT Protocol: " + result.uri);
        } catch (e) {
          console.warn("AT Protocol publish failed (non-fatal):", e.message);
        }
      }
      wasmSession.receiveTable(tableCbor);
      addLog("Table record broadcast. Agents processing...");
      refreshGameView();
    }
  }

  function replayGameState(players, history) {
    if (!history || history.length === 0) return;
    initWasmGame(players);
    if (!wasmSession) return;
    addLog(`Replaying ${history.length} action(s)...`);
    for (const entry of history) {
      try {
        if (entry.action.type === "wasm_table") {
          wasmSession.receiveTable(base64ToUint8Array(entry.action.cbor));
        } else if (entry.action.type === "wasm_action") {
          wasmSession.receiveAction(base64ToUint8Array(entry.action.cbor));
        }
      } catch (e) {
        console.warn("Replay error:", e.message);
      }
    }
    refreshGameView();
    addLog("Rejoined game!");
  }

  function receiveRemoteAction(fromPlayerId, action) {
    if (!wasmSession) return;

    if (action.type === "wasm_table") {
      if (wasmSession) {
        wasmSession.destroy();
        wasmSession = null;
      }
      _hadCards = false;
      initWasmGame(Object.values(gameState.players));
      try {
        const cbor = base64ToUint8Array(action.cbor);
        const out = wasmSession.receiveTable(cbor);
        addLog(`Table received from ${fromPlayerId}, produced ${out.length} action(s)`);
        refreshGameView();
      } catch (e) {
        console.error("Failed to process WASM table:", e);
      }
    } else if (action.type === "wasm_action") {
      try {
        const cbor = base64ToUint8Array(action.cbor);
        const out = wasmSession.receiveAction(cbor);

        if (out.length > 0) {
          addLog(`Processed action from ${fromPlayerId}, produced ${out.length} action(s)`);
        }
        refreshGameView();
      } catch (e) {
        console.error("Failed to process WASM action:", e);
      }
    } else if (action.type === "phase_change") {
      gameState = {
        ...gameState,
        phase: action.phase,
        pot: action.pot,
        currentBet: action.currentBet,
        currentPlayer: action.currentPlayer,
        actedThisRound: action.actedThisRound || [],
      };
    }
  }

  function refreshGameView() {
    if (!wasmSession || !gameState) return;

    const holeRaw = wasmSession.holeCards;
    const commRaw = wasmSession.communityCards;

    if (holeRaw.length > 0) {
      decryptedHoleCards = { [ourPlayerId]: holeRaw };
      _hadCards = true;
    }
    decryptedCommunityCards = commRaw;

    // Sync chips/pot/actionOn from WASM
    const gs = wasmSession.gameState;
    if (gs) {
      gameState.pot = gs.pot ?? gameState.pot;
      gameState.currentBet = gs.currentBet ?? gameState.currentBet;
      if (gs.players)
        for (const p of gs.players) {
          const pid = gameState.playerOrder[p.seat];
          if (pid && gameState.players[pid]) {
            gameState.players[pid].chips = p.chips;
            gameState.players[pid].bet = p.bet;
            gameState.players[pid].folded = p.folded;
          }
        }
      if (gs.actionOn != null && gameState.playerOrder[gs.actionOn]) {
        gameState.currentPlayer = gameState.playerOrder[gs.actionOn];
      }
    }

    // Derive UI phase from agent phase + community card count
    const wasmPhase = wasmSession.phase;
    const commLen = commRaw.length;
    let uiPhase = gameState.phase;
    if (wasmPhase === "Showdown" || wasmPhase === "Complete") {
      uiPhase = GAME_PHASES.SHOWDOWN;
    } else if (commLen >= 5) uiPhase = GAME_PHASES.RIVER;
    else if (commLen >= 4) uiPhase = GAME_PHASES.TURN;
    else if (commLen >= 3) uiPhase = GAME_PHASES.FLOP;
    else if (wasmPhase === "Betting") uiPhase = GAME_PHASES.PREFLOP;
    gameState = { ...gameState, phase: uiPhase };

    // Hand complete — announce winner once
    if (wasmSession.isComplete && !_handOver) {
      _handOver = true;
      const winnerId = findWinner();
      if (winnerId) {
        const winnerName = gameState.players[winnerId]?.name || winnerId;
        addLog(`${winnerName} wins ${gameState.pot} chips!`);
      } else {
        addLog("Hand complete.");
      }
    }

    // Betting / new-hand controls
    wasmSession.checkStatus();
    if (_handOver && ourPlayerId === gameState.playerOrder[0]) {
      // Dealer drives the next hand.
      isOurTurn = true;
      availableActions = [{ type: "new_hand", label: "DEAL NEW HAND" }];
      raiseContext = null;
    } else if (wasmSession.needsBet) {
      isOurTurn = true;
      availableActions = mapBetOptions(wasmSession.betOptions || []);
      const ourChips = (gameState?.players || {})[ourPlayerId]?.chips ?? 1000;
      const minRaise = gameState?.bigBlind ?? 2;
      raiseContext = {
        min: minRaise,
        max: ourChips,
        pot: gameState?.pot || 0,
        quickAmounts: buildQuickAmounts(gameState?.pot || 0, minRaise, ourChips),
      };
    } else {
      isOurTurn = false;
      availableActions = [];
      raiseContext = null;
    }
  }

  function mapBetOptions(opts) {
    return opts
      .map((opt) => {
        if (opt === "Fold") return { type: "fold", label: "FOLD" };
        if (opt === "Check") return { type: "check", label: "CHECK" };
        if (opt === "Call") return { type: "call", label: "CALL" };
        if (opt === "AllIn") return { type: "allIn", label: "ALL IN" };
        if (opt && typeof opt === "object" && "Raise" in opt) {
          return { type: "raise", label: "RAISE", amount: opt.Raise };
        }
        return null;
      })
      .filter(Boolean);
  }

  function buildQuickAmounts(pot, min, max) {
    const out = [];
    for (const [label, amt] of [
      ["1/3 POT", Math.floor(pot / 3)],
      ["1/2 POT", Math.floor(pot / 2)],
      ["POT", pot],
    ]) {
      if (amt > min && amt <= max) out.push({ label, amount: amt });
    }
    return out;
  }

  function findWinner() {
    // Win-by-fold: only one non-folded player remains.
    // (Showdown winner determination would require exposing other players'
    // revealed hole cards from the WASM agent, which the API doesn't yet support.)
    const remaining = gameState.playerOrder.filter((pid) => !gameState.players[pid]?.folded);
    return remaining.length === 1 ? remaining[0] : null;
  }

  // ─── Player Actions ───────────────────────────────────────────────
  function handleAction(action) {
    if (!wasmSession) return;

    const betMap = {
      fold: "fold",
      check: "check",
      call: "call",
      allIn: "allIn",
    };

    if (action.type === "new_hand") {
      restartHand();
      return;
    }
    let betStr =
      betMap[action.type] || (typeof action.type === "string" ? action.type.toLowerCase() : null);
    if (action.type === "raise" || action.type === "Raise") {
      betStr = `raise:${action.amount || gameState?.bigBlind || 2}`;
    }

    if (betStr) {
      try {
        addLog(`You ${action.type}${action.amount ? " " + action.amount : ""}`);
        wasmSession.bet(betStr);
        refreshGameView();

        // After betting, if we have cards but no bet needed, hand is over
      } catch (e) {
        console.error("Bet failed:", e);
        error = "Action failed: " + e.message;
      }
    }
  }

  // ─── Helpers ──────────────────────────────────────────────────────

  function restartHand() {
    if (!gameState) return;
    const players = Object.values(gameState.players);
    initWasmGame(players);
  }

  function uint8ToBase64(buffer) {
    const bytes = new Uint8Array(buffer);
    let binary = "";
    for (let i = 0; i < bytes.byteLength; i++) {
      binary += String.fromCharCode(bytes[i]);
    }
    return btoa(binary);
  }

  function base64ToUint8Array(b64) {
    const binary = atob(b64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    return bytes;
  }

  function leave() {
    roomClient.leave();
    onLeaveRoom();
  }

  let copied = $state(false);
  async function copyRoomId() {
    try {
      await navigator.clipboard.writeText(roomId);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch (e) {
      console.warn("clipboard unavailable:", e);
    }
  }
</script>

<div class="game-room">
  <header>
    <div class="room-header">
      {#if userAvatar}
        <div class="avatar-circle">
          <img src={userAvatar} alt="avatar" />
        </div>
      {/if}
      <span class="handle-name">{playerName}</span>
      <span class="connection {connected ? 'online' : 'offline'}">
        {connected ? "●" : "○"}
      </span>
      <button
        class="room-id"
        onclick={copyRoomId}
        title="Click to copy room ID"
        data-testid="copy-room-id"
      >
        room: <code>{roomId}</code>
        <span class="copy-hint">{copied ? "✓ copied" : "copy"}</span>
      </button>
    </div>
    <div class="header-actions">
      {#if spectating}
        <span class="spectating-badge">👁 Watching</span>
      {/if}
      {#if gameState && ourSeat >= 0}
        <span class="phase-label">{gamePhase}</span>
      {/if}
      <button class="btn leave" onclick={leave}>Leave Room</button>
    </div>
  </header>

  {#if error}
    <div class="error-banner">{error}</div>
  {/if}

  <div class="main-area">
    {#if !gameState}
      <!-- Waiting room -->
      <div class="waiting-room">
        <h3>{spectating ? "Spectating Lobby" : "Waiting Room"}</h3>

        {#if spectating}
          <div class="spectator-waiting">
            <p class="hint">You are watching this room. The game will appear when it starts.</p>
            <div class="seat-grid">
              {#each Array(8) as _, i}
                <div class="seat-btn" class:occupied={room.players.some((p) => p.seat === i)}>
                  {#if room.players.some((p) => p.seat === i)}
                    {@const occupant = room.players.find((p) => p.seat === i)}
                    <span class="seat-name">{occupant.name || "Player"}</span>
                    {#if occupant.ready}
                      <span class="ready-badge">✓</span>
                    {/if}
                  {:else}
                    <span class="seat-empty">Seat {i + 1}</span>
                  {/if}
                </div>
              {/each}
            </div>
          </div>
        {:else}
          <div class="available-seats">
            <p class="hint">Click a seat to sit down</p>
            <div class="seat-grid">
              {#each Array(8) as _, i}
                <button
                  class="seat-btn"
                  class:occupied={room.players.some((p) => p.seat === i)}
                  class:ours={ourSeat === i}
                  onclick={() => sitAtSeat(i)}
                  disabled={room.players.some((p) => p.seat === i && p.id !== playerId)}
                >
                  {#if room.players.some((p) => p.seat === i)}
                    {@const occupant = room.players.find((p) => p.seat === i)}
                    <span class="seat-name"
                      >{occupant.id === playerId ? "YOU" : occupant.name || "Player"}</span
                    >
                    {#if occupant.ready}
                      <span class="ready-badge">✓</span>
                    {/if}
                  {:else if ourSeat === i}
                    <!-- Optimistic: show we selected this seat before server confirms -->
                    <span class="seat-name ours-name">YOU</span>
                    <span class="seat-sub">selected</span>
                  {:else}
                    <span class="seat-empty">Seat {i + 1}</span>
                  {/if}
                </button>
              {/each}
            </div>
          </div>

          {#if ourSeat >= 0}
            <button class="btn ready-btn" onclick={readyUp}> Ready Up </button>
          {/if}
        {/if}

        <p class="player-count">
          {room.players.length} player{room.players.length !== 1 ? "s" : ""} in room (need at least 2)
          {#if room.spectatorCount > 0}
            · {room.spectatorCount} watching
          {/if}
        </p>
      </div>
    {:else}
      <!-- Game active -->
      <div class="game-layout">
        <div class="table-wrapper">
          <PokerTable
            players={playerMap}
            {playerOrder}
            {playerDids}
            {handleMap}
            holeCards={decryptedHoleCards}
            communityCards={decryptedCommunityCards}
            pot={gameState.pot}
            currentPlayer={gameState.currentPlayer}
            {ourPlayerId}
            {gamePhase}
            showAllCards={gamePhase === "showdown"}
          />
        </div>

        <div class="bottom-panel">
          {#if !spectating}
            <ActionBar
              actions={availableActions}
              raise={raiseContext}
              onAction={handleAction}
              {isOurTurn}
            />

            {#if !isOurTurn && gameState}
              <div class="waiting-turn">
                Waiting for {gameState.players[gameState.currentPlayer]?.name || "..."} to act...
              </div>
            {/if}
          {:else}
            <div class="spectating-turn">
              👁 Spectating — {gameState.players[gameState.currentPlayer]?.name || "..."}'s turn
            </div>
          {/if}

          <GameLog events={logEvents} />
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .game-room {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    background: #ffffff;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.75rem 1.5rem;
    background: #ffffff;
    border-bottom: 3px solid #1a1a1a;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .room-header {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .avatar-circle {
    width: 28px;
    height: 28px;
    border: 2px solid #1a1a1a;
    border-radius: 50%;
    overflow: hidden;
    flex-shrink: 0;
    image-rendering: pixelated;
    image-rendering: crisp-edges;
  }
  .avatar-circle img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .handle-name {
    font-size: 0.5rem;
    color: #1a1a1a;
  }
  .connection {
    font-size: 0.4rem;
  }
  .connection.online {
    color: #1a1a1a;
  }
  .connection.offline {
    color: #c0392b;
  }
  .room-id {
    font-family: inherit;
    font-size: 0.4rem;
    background: #ffffff;
    color: #1a1a1a;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    padding: 0.25rem 0.5rem;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    box-shadow: 2px 2px 0 #1a1a1a;
    transition: all 0.1s;
  }
  .room-id code {
    font-family: inherit;
    color: #c0392b;
    letter-spacing: 1px;
  }
  .copy-hint {
    font-size: 0.32rem;
    opacity: 0.6;
    letter-spacing: 1px;
  }
  .room-id:hover {
    transform: translate(1px, 1px);
    box-shadow: 1px 1px 0 #1a1a1a;
  }
  .room-id:active {
    transform: translate(2px, 2px);
    box-shadow: none;
  }
  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .phase-label {
    font-size: 0.4rem;
    color: #c0392b;
    letter-spacing: 2px;
  }
  .spectating-badge {
    font-size: 0.4rem;
    color: #1a1a1a;
    background: #f0f0f0;
    padding: 0.2rem 0.5rem;
    border: 2px solid #1a1a1a;
    letter-spacing: 1px;
  }
  .btn {
    padding: 0.5rem 1rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    font-family: inherit;
    font-size: 0.4rem;
    cursor: pointer;
    letter-spacing: 1px;
    background: #ffffff;
    color: #1a1a1a;
    box-shadow: 3px 3px 0 #1a1a1a;
    transition: all 0.1s;
  }
  .btn:hover:not(:disabled) {
    transform: translate(2px, 2px);
    box-shadow: 1px 1px 0 #1a1a1a;
  }
  .btn:active:not(:disabled) {
    transform: translate(3px, 3px);
    box-shadow: none;
  }
  .leave {
    background: #ffffff;
    color: #1a1a1a;
    border: 2px solid #1a1a1a;
  }
  .leave:hover {
    background: #c0392b;
    color: #ffffff;
  }
  .ready-btn {
    background: #1a1a1a;
    color: #ffffff;
    padding: 0.75rem 3rem;
    font-size: 0.5rem;
    margin-top: 1rem;
  }
  .error-banner {
    background: #c0392b;
    color: #ffffff;
    padding: 0.5rem;
    text-align: center;
    font-size: 0.45rem;
  }
  .main-area {
    flex: 1;
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
  }
  .waiting-room {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1rem;
  }
  .waiting-room h3 {
    font-size: 0.7rem;
    color: #1a1a1a;
  }
  .hint {
    font-size: 0.45rem;
    color: #1a1a1a;
    opacity: 0.6;
    margin-bottom: 0.5rem;
  }
  .available-seats {
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .seat-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 0.5rem;
    max-width: 400px;
  }
  .seat-btn {
    padding: 0.6rem 0.8rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    background: #ffffff;
    color: #1a1a1a;
    font-family: inherit;
    font-size: 0.4rem;
    cursor: pointer;
    transition: all 0.1s;
    min-width: 90px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  .seat-btn:hover:not(:disabled) {
    border-color: #c0392b;
    box-shadow: 3px 3px 0 #c0392b;
    transform: translate(-1px, -1px);
  }
  .seat-btn:disabled {
    cursor: not-allowed;
    opacity: 0.4;
  }
  .seat-btn.occupied {
    background: #1a1a1a;
    color: #ffffff;
  }
  .seat-btn.ours {
    border-color: #c0392b;
    background: #c0392b;
    color: #ffffff;
    box-shadow:
      4px 4px 0 #8b0000,
      0 0 12px rgba(192, 57, 43, 0.5);
    transform: translate(-2px, -2px);
    position: relative;
  }
  .seat-btn.ours::after {
    content: "▼";
    position: absolute;
    bottom: -14px;
    left: 50%;
    transform: translateX(-50%);
    font-size: 0.5rem;
    color: #c0392b;
    filter: drop-shadow(0 2px 0 #8b0000);
  }
  .seat-name {
    font-size: 0.35rem;
  }
  .seat-sub {
    font-size: 0.28rem;
    color: #ffffff;
    opacity: 0.7;
    letter-spacing: 1px;
  }
  .ready-badge {
    color: #c0392b;
    font-size: 0.45rem;
  }
  .seat-empty {
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.4;
  }
  .player-count {
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.5;
  }
  .game-layout {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .table-wrapper {
    flex: 1;
    display: flex;
    align-items: center;
  }
  .bottom-panel {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    max-width: 750px;
    margin: 0 auto;
    width: 100%;
  }
  .waiting-turn {
    text-align: center;
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.5;
    padding: 0.5rem;
  }
  .spectating-turn {
    text-align: center;
    font-size: 0.4rem;
    color: #1a1a1a;
    padding: 0.5rem;
    border: 2px solid #1a1a1a;
    opacity: 0.7;
  }
</style>
