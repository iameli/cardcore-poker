<script>
  import { onMount } from 'svelte';
  import PokerTable from './PokerTable.svelte';
  import ActionBar from './ActionBar.svelte';
  import GameLog from './GameLog.svelte';
  import { roomClient } from '../lib/room.js';
  import { PlayerSession, buildTableCbor, generateSeed } from '../lib/game-session.js';
  import { initWasm } from '../lib/cardcore-wasm.js';
  import {
    evaluateHand,
    compareHands,
    GAME_PHASES,
  } from '../lib/poker-engine.js';

  let { session, roomId, spectating = false, onLeaveRoom } = $props();

  // ─── Local State ──────────────────────────────────────────────────
  let playerId = $state('');
  let playerName = $state('');
  let room = $state({ players: [], id: '' });
  let gameState = $state(null);
  let logEvents = $state([]);
  let connected = $state(false);
  let error = $state('');
  let ourSeat = $state(-1);

  // WASM session
  let wasmSession = null;
  let _seed = null;
  let decryptedHoleCards = $state({});
  let decryptedCommunityCards = $state([]);
  let availableActions = $state([]);
  let raiseContext = $state(null);
  let isOurTurn = $state(false);
  let _pendingActions = [];

  // ─── Derived ──────────────────────────────────────────────────────
  const ourPlayerId = $derived(playerId);
  const gamePhase = $derived(gameState?.phase || 'idle');
  const playerMap = $derived(gameState?.players || {});
  const playerOrder = $derived(gameState?.playerOrder || []);
  const playerDids = $derived.by(() => {
    const map = {};
    for (const p of (room?.players || [])) {
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
    _pid = `@${session?.handle || session?.name || 'Player'}-${Math.random().toString(36).slice(2, 8)}`;
    return _pid;
  }
  let _wired = false;

  onMount(async () => {
    await initWasm();

    const pid = ensurePlayerId();
    const pName = session?.handle || session?.name || 'Player';
    playerName = pName;
    playerId = pid;

    const realDid = session?.session ? (session.did || null) : null;
    addLog('Connecting to room...');
    roomClient.connect(pid, roomId, pName, realDid);

    if (!_wired) {
      _wired = true;

      roomClient.on('connected', () => {
        connected = true;
        addLog('Connected to room!');
      });

      roomClient.on('disconnected', () => {
        connected = false;
        addLog('Disconnected. Reconnecting...');
      });

      roomClient.on('room_update', (data) => {
        room = data.room;
        const our = data.room.players.find(p => p.id === playerId);
        if (our) ourSeat = our.seat;
      });

      roomClient.on('game_start', (data) => {
        addLog('Game starting!');
        initWasmGame(data.players);
      });

      roomClient.on('game_action', (data) => {
        if (data.playerId !== ourPlayerId) {
          receiveRemoteAction(data.playerId, data.action);
        }
      });
    }

    return () => {
      if (wasmSession) { wasmSession.destroy(); wasmSession = null; }
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
    addLog('Ready!');
  }

  // ─── WASM Game Init ───────────────────────────────────────────────
  async function initWasmGame(players) {
    if (spectating || !players || players.length < 2) return;

    const playerIds = players.map(p => p.id);
    const isDealer = ourPlayerId === playerIds[0];

    // Generate our seed and create session
    if (!_seed) _seed = generateSeed();
    if (!wasmSession) {
      // Use DID if available, otherwise our playerId
      const did = session?.did || ourPlayerId;
      wasmSession = new PlayerSession({
        did,
        seed: _seed,
        send: (cbor) => {
          // Broadcast CBOR action via WebSocket
          roomClient.sendAction({
            type: 'wasm_action',
            cbor: arrayBufferToBase64(cbor.buffer || cbor),
          });
        },
      });
    }

    // Build synthetic gameState from room data
    const playersObj = {};
    for (const p of players) {
      playersObj[p.id] = {
        id: p.id,
        name: p.name,
        did: p.did || null,
        chips: 1000,
        bet: 0,
        folded: false,
        seat: p.seat,
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
      addLog('Dealer: initializing WASM table...');
      // Build table CBOR with DIDs
      const dids = playerIds.map(id => session?.did || playerDids[id] || id);
      const tableCbor = buildTableCbor({
        players: dids,
        startingChips: 1000,
        smallBlind: 10,
      });
      // Broadcast table CBOR to all players
      roomClient.sendAction({
        type: 'wasm_action',
        cbor: arrayBufferToBase64(tableCbor.buffer),
      });
      // Feed to our session
      const actions = wasmSession.receiveTable(tableCbor);
      _pendingActions = actions;
      addLog('Table record broadcast. Agents processing...');
      refreshGameView();
    }
  }

  function receiveRemoteAction(fromPlayerId, action) {
    if (!wasmSession) return;

    if (action.type === 'wasm_action') {
      try {
        const cbor = base64ToUint8Array(action.cbor);
        const out = wasmSession.receiveAction(cbor);
        if (out.length > 0) {
          addLog(`Processed action from ${fromPlayerId}, produced ${out.length} response(s)`);
        }
        refreshGameView();
      } catch (e) {
        console.error('Failed to process WASM action:', e);
      }
    } else if (action.type === 'fold' || action.type === 'check' ||
               action.type === 'call' || action.type === 'raise' ||
               action.type === 'allIn') {
      // Legacy betting action — forward to WASM agent
      try {
        const betAction = action.type === 'raise'
          ? `raise:${action.amount || gameState?.bigBlind || 2}`
          : action.type === 'allIn' ? 'allIn' : action.type;
        wasmSession.bet(betAction);
        refreshGameView();
      } catch (e) {
        console.error('Bet action failed:', e);
      }
    } else if (action.type === 'phase_change') {
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

    // Get cards from WASM
    const holeRaw = wasmSession.holeCards;
    const commRaw = wasmSession.communityCards;

    // Update hole cards (indexed by our player ID)
    if (holeRaw.length > 0) {
      decryptedHoleCards = { [ourPlayerId]: holeRaw };
    }
    decryptedCommunityCards = commRaw;

    // Check betting status
    const status = wasmSession.checkStatus();
    if (wasmSession.needsBet) {
      isOurTurn = true;
      const opts = wasmSession.betOptions || [];
      availableActions = opts.map(opt => {
        if (typeof opt === 'string') return { type: opt, label: opt.toUpperCase() };
        if (opt === 'Fold') return { type: 'fold', label: 'FOLD' };
        if (opt === 'Check') return { type: 'check', label: 'CHECK' };
        if (opt === 'Call') return { type: 'call', label: 'CALL' };
        if (opt === 'AllIn') return { type: 'allIn', label: 'ALL IN' };
        // Raise option
        return { type: 'raise', label: String(opt) };
      });
      raiseContext = { min: 2, max: 1000, pot: gameState?.pot || 0, quickAmounts: [] };
    } else {
      isOurTurn = false;
      availableActions = [];
      raiseContext = null;
    }

    // Update phase in gameState
    if (wasmSession.phase === 'betting') {
      gameState = { ...gameState, phase: GAME_PHASES.PREFLOP };
    }
  }

  // ─── Player Actions ───────────────────────────────────────────────
  function handleAction(action) {
    if (!wasmSession) return;

    const betMap = {
      fold: 'fold',
      check: 'check',
      call: 'call',
      allIn: 'allIn',
    };

    let betStr = betMap[action.type];
    if (action.type === 'raise') {
      betStr = `raise:${action.amount || gameState?.bigBlind || 2}`;
    }

    if (betStr) {
      try {
        addLog(`You ${action.type}${action.amount ? ' ' + action.amount : ''}`);
        wasmSession.bet(betStr);
        // Also broadcast as legacy action for non-WASM clients
        roomClient.sendAction(action);
        refreshGameView();
      } catch (e) {
        console.error('Bet failed:', e);
        error = 'Action failed: ' + e.message;
      }
    }
  }

  // ─── Helpers ──────────────────────────────────────────────────────
  function arrayBufferToBase64(buffer) {
    const bytes = new Uint8Array(buffer);
    let binary = '';
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
    if (wasmSession) { wasmSession.destroy(); wasmSession = null; }
    roomClient.leave();
    onLeaveRoom();
  }</script>

<div class="game-room">
  <header>
    <div class="room-header">
      <span class="room-badge">Room: {roomId}</span>
      <span class="connection {connected ? 'online' : 'offline'}">
        {connected ? '● Connected' : '○ Disconnected'}
      </span>
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
        <h3>{spectating ? 'Spectating Lobby' : 'Waiting Room'}</h3>

        {#if spectating}
          <div class="spectator-waiting">
            <p class="hint">You are watching this room. The game will appear when it starts.</p>
            <div class="seat-grid">
              {#each Array(8) as _, i}
                <div class="seat-btn" class:occupied={room.players.some(p => p.seat === i)}>
                  {#if room.players.some(p => p.seat === i)}
                    {@const occupant = room.players.find(p => p.seat === i)}
                    <span class="seat-name">{occupant.name || 'Player'}</span>
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
                  class:occupied={room.players.some(p => p.seat === i)}
                  class:ours={ourSeat === i}
                  onclick={() => sitAtSeat(i)}
                  disabled={room.players.some(p => p.seat === i && p.id !== playerId)}
                >
                  {#if room.players.some(p => p.seat === i)}
                    {@const occupant = room.players.find(p => p.seat === i)}
                    <span class="seat-name">{occupant.id === playerId ? 'YOU' : (occupant.name || 'Player')}</span>
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
            <button class="btn ready-btn" onclick={readyUp}>
              Ready Up
            </button>
          {/if}
        {/if}

        <p class="player-count">
          {room.players.length} player{room.players.length !== 1 ? 's' : ''} in room
          (need at least 2)
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
            playerOrder={playerOrder}
            playerDids={playerDids}
            holeCards={decryptedHoleCards}
            communityCards={decryptedCommunityCards}
            pot={gameState.pot}
            currentPlayer={gameState.currentPlayer}
            ourPlayerId={ourPlayerId}
            gamePhase={gamePhase}
            showAllCards={gamePhase === 'showdown'}
          />
        </div>

        <div class="bottom-panel">
          {#if !spectating}
            <ActionBar
              actions={availableActions}
              raise={raiseContext}
              onAction={handleAction}
              isOurTurn={isOurTurn}
            />

            {#if !isOurTurn && gameState}
              <div class="waiting-turn">
                Waiting for {gameState.players[gameState.currentPlayer]?.name || '...'} to act...
              </div>
            {/if}
          {:else}
            <div class="spectating-turn">
              👁 Spectating — {gameState.players[gameState.currentPlayer]?.name || '...'}'s turn
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
    gap: 1rem;
  }
  .room-badge {
    font-size: 0.45rem;
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
    box-shadow: 4px 4px 0 #8b0000, 0 0 12px rgba(192, 57, 43, 0.5);
    transform: translate(-2px, -2px);
    position: relative;
  }
  .seat-btn.ours::after {
    content: '▼';
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
