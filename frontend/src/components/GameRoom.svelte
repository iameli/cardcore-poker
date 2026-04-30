<script>
  import { onMount } from 'svelte';
  import PokerTable from './PokerTable.svelte';
  import ActionBar from './ActionBar.svelte';
  import GameLog from './GameLog.svelte';
  import { roomClient } from '../lib/room.js';
  import {
    generateKeyPair,
    encodePublicKey,
    boxCard,
    unboxCard,
    createDeck,
    SUITS,
    RANKS,
  } from '../lib/mental-poker.js';
  import {
    evaluateHand,
    compareHands,
    getAvailableActions,
    GAME_PHASES,
    ACTIONS,
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

  // Mental poker state
  let keyPair = $state(null);
  let keyPairs = $state({});
  let encryptedDeck = $state([]);
  let decryptedHoleCards = $state({});
  let decryptedCommunityCards = $state([]);

  // ─── Derived ──────────────────────────────────────────────────────
  const ourPlayerId = $derived(playerId);
  const gamePhase = $derived(gameState?.phase || 'idle');
  const isOurTurn = $derived(gameState?.currentPlayer === ourPlayerId);
  const _actionData = $derived(
    gameState && isOurTurn ? getAvailableActions(gameState, ourPlayerId) : { actions: [], raise: null }
  );
  const availableActions = $derived(_actionData.actions);
  const raiseContext = $derived(_actionData.raise);
  const playerMap = $derived(gameState?.players || {});
  const playerOrder = $derived(gameState?.playerOrder || []);
  const playerDids = $derived.by(() => {
    const fromRoom = Object.fromEntries(
      (room?.players || [])
        .filter(p => p.did)
        .map(p => [p.id, p.did])
    );
    const fromGame = Object.fromEntries(
      Object.values(gameState?.players || {})
        .filter(p => p.did)
        .map(p => [p.id, p.did])
    );
    const merged = { ...fromRoom, ...fromGame };
    const keys = Object.keys(merged);
    if (keys.length > 0) {
      console.log('[playerDids] map:', JSON.stringify(merged));
      console.log('[playerDids] playerOrder:', JSON.stringify(playerOrder));
      for (const pid of playerOrder) {
        console.log(`[playerDids] lookup playerOrder id="${pid}" → did="${merged[pid] || 'NOT FOUND'}"`);
      }
    }
    return merged;
  });

  // ─── Initialize ───────────────────────────────────────────────────
  // Generate stable playerId ONCE from session data.
  // Must NOT depend on $state playerId — that creates an infinite
  // reactivity loop (write $state → const invalidates → effect re-runs).
  let _pid = null;
  function ensurePlayerId() {
    if (_pid) return _pid;
    _pid = `@${session?.handle || session?.name || 'Player'}-${Math.random().toString(36).slice(2, 8)}`;
    return _pid;
  }
  let _wired = false;

  onMount(() => {
    const pid = ensurePlayerId();
    const pName = session?.handle || session?.name || 'Player';
    playerName = pName;
    playerId = pid;

    // Only send real DIDs (OAuth sessions); demo identities have fake DIDs
    const realDid = session?.session ? (session.did || null) : null;
    addLog('Connecting to room...');
    roomClient.connect(pid, roomId, pName, realDid);

    if (!_wired) {
      _wired = true;

      roomClient.on('connected', () => {
          connected = true;
          console.log('[WS] Connected to server');
          addLog('Connected to room!');
        });

      roomClient.on('disconnected', () => {
          connected = false;
          console.log('[WS] Disconnected from server');
          addLog('Disconnected. Reconnecting...');
        });

      roomClient.on('room_update', (data) => {
          console.log(`[ROOM_UPDATE] received:`, JSON.stringify(data.room));
          room = data.room;
          const ourPlayer = data.room.players.find((p) => p.id === playerId);
          if (ourPlayer) {
            console.log(`[ROOM_UPDATE] Found our player at seat ${ourPlayer.seat}`);
            ourSeat = ourPlayer.seat;
          } else {
            console.log(`[ROOM_UPDATE] Our player not found (playerId=${playerId})`);
          }
        });

      roomClient.on('game_start', (data) => {
          console.log('[GAME] Game starting with players:', data.players?.length);
          addLog('Game starting!');
          initGame(data.players);
        });

      roomClient.on('game_action', (data) => {
        if (data.playerId !== ourPlayerId) {
          handleRemoteAction(data.playerId, data.action);
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
    // Keep last 50
    if (logEvents.length > 50) {
      logEvents = logEvents.slice(-50);
    }
  }

  // ─── Sit / Ready ──────────────────────────────────────────────────
  function sitAtSeat(seatIndex) {
    if (ourSeat === seatIndex) {
      console.log(`[SIT] Already at seat ${seatIndex + 1}, skipping`);
      return;
    }
    console.log(`[SIT] Clicked seat ${seatIndex + 1}, ourSeat was ${ourSeat}, sending sit...`);
    ourSeat = seatIndex;
    roomClient.sit(seatIndex);
    addLog(`Sat at seat ${seatIndex + 1}`);
  }

  function readyUp() {
    roomClient.ready();
    addLog('Ready!');
  }

  // ─── Game Initialization ──────────────────────────────────────────
  async function initGame(players) {
    if (!spectating) {
      // Generate our keypair (players only)
      keyPair = generateKeyPair();
      addLog('Generated keypair for mental poker');

      // Add our own public key to keyPairs so the all-keys check passes
      keyPairs = {
        ...keyPairs,
        [ourPlayerId]: {
          publicKey: keyPair.publicKey,
          playerId: ourPlayerId,
        },
      };

      // Exchange public keys with all players
      broadcastKeyExchange();
    }

    // Wait for all public keys to arrive, then dealer sets up the shared deck
    const playerIds = players.map(p => p.id);
    const isDealer = ourPlayerId === playerIds[0];

    // Poll until we have all keys (non-blocking via setInterval)
    const checkKeys = setInterval(() => {
      const allKeysPresent = playerIds.every(id => keyPairs[id]);
      if (allKeysPresent || spectating) {
        clearInterval(checkKeys);
        if (isDealer && !spectating) {
          // Only the dealer creates and distributes the shared deck
          addLog('All keys received — dealing as dealer');
          setupGame(players);
        } else if (!spectating) {
          addLog('All keys received — waiting for dealer to broadcast game state');
        } else {
          // Spectators wait for game state broadcast
          addLog('Spectating — waiting for game state');
        }
      }
    }, 100);
  }

  function broadcastKeyExchange() {
    roomClient.sendAction({
      type: 'key_exchange',
      publicKey: keyPair.publicKey,
      playerId: ourPlayerId,
    });
  }

  // ─── Game Setup (DEALER ONLY) ─────────────────────────────────────
  function setupGame(players) {
    const playersObj = {};
    const order = [];
    const startingChips = 1000;

    for (const p of players) {
      playersObj[p.id] = {
        id: p.id,
        name: p.name,
        did: p.did || null,
        chips: startingChips,
        bet: 0,
        folded: false,
        seat: p.seat,
      };
      order.push(p.id);
    }

    // Create and shuffle the deck (dealer does this ONCE for everyone)
    const deck = createDeck();
    const shuffled = [...deck].sort(() => Math.random() - 0.5);

    // Build the local game state (dealer's own view)
    gameState = {
      players: playersObj,
      playerOrder: order,
      deck: shuffled,
      deckIndex: 0,
      phase: GAME_PHASES.PREFLOP,
      pot: 0,
      currentBet: 2,
      bigBlind: 2,
      currentPlayer: order[0],
      actedThisRound: [],
    };

    // Deal hole cards (this sets gameState.holeCards and communityCards)
    dealHoleCards();

    // Encrypt hole cards for each player and broadcast the shared state
    broadcastGameState();
  }

  /**
   * Encrypts and broadcasts the initial game state so all clients share the same deck.
   * DEALER ONLY.
   */
  function broadcastGameState() {
    const state = gameState;
    const encryptedHoleCards = {};

    // Encrypt each player's hole cards with their public key
    for (const pid of state.playerOrder) {
      const cards = state.holeCards[pid] || [];
      const recipientKey = keyPairs[pid]?.publicKey;
      if (recipientKey && keyPair) {
        encryptedHoleCards[pid] = cards.map(card =>
          boxCard(card, recipientKey, keyPair.secretKey)
        );
      } else {
        // Fallback: if we don't have their key, send raw (for now)
        encryptedHoleCards[pid] = cards;
      }
    }

    // Encrypt community cards with each player's key (so all can decrypt)
    const encryptedCommunity = state.communityCards.map(card => {
      const encryptions = {};
      for (const pid of state.playerOrder) {
        const recipientKey = keyPairs[pid]?.publicKey;
        if (recipientKey && keyPair) {
          encryptions[pid] = boxCard(card, recipientKey, keyPair.secretKey);
        }
      }
      return encryptions;
    });

    // Broadcast the shared game state to all clients
    roomClient.sendAction({
      type: 'game_state_init',
      players: state.players,
      playerOrder: state.playerOrder,
      deckIndex: state.deckIndex,
      phase: state.phase,
      pot: state.pot,
      currentBet: state.currentBet,
      bigBlind: state.bigBlind,
      currentPlayer: state.currentPlayer,
      actedThisRound: state.actedThisRound || [],
      encryptedHoleCards,
      encryptedCommunity,
      dealerPublicKey: keyPair.publicKey,
    });

    addLog('Broadcast encrypted game state to all players');
  }

  function handleRemoteAction(fromPlayerId, action) {
    switch (action.type) {
      case 'key_exchange': {
        keyPairs = {
          ...keyPairs,
          [fromPlayerId]: {
            publicKey: action.publicKey,
            playerId: fromPlayerId,
          },
        };
        addLog(`Got public key from ${fromPlayerId}`);
        break;
      }
      case 'game_state_init': {
        // Non-dealer clients: receive the shared game state from the dealer
        if (!keyPair) {
          addLog('Received game state but no keypair yet — delaying');
          setTimeout(() => handleRemoteAction(fromPlayerId, action), 200);
          return;
        }
        addLog('Received encrypted game state from dealer — decrypting');

        // Decrypt our hole cards
        const holeCards = {};
        for (const pid of action.playerOrder) {
          const encryptedCards = action.encryptedHoleCards[pid] || [];
          if (pid === ourPlayerId) {
            // Decrypt our own cards using our secret key + dealer's public key
            holeCards[pid] = encryptedCards.map(enc => {
              if (enc && enc.nonce && enc.ciphertext) {
                return unboxCard(enc, keyPair.secretKey, action.dealerPublicKey);
              }
              // Fallback: raw card
              return enc;
            });
          } else {
            // Store nulls for opponents (we can't decrypt their cards)
            holeCards[pid] = encryptedCards.map(() => null);
          }
        }
        decryptedHoleCards = holeCards;

        // Decrypt community cards
        const community = action.encryptedCommunity.map(encryptions => {
          // Try to decrypt with our key
          const ourEnc = encryptions[ourPlayerId];
          if (ourEnc && ourEnc.nonce && ourEnc.ciphertext) {
            return unboxCard(ourEnc, keyPair.secretKey, action.dealerPublicKey);
          }
          return null;
        });
        decryptedCommunityCards = community;

        // Set up shared game state
        gameState = {
          players: action.players,
          playerOrder: action.playerOrder,
          deckIndex: action.deckIndex,
          phase: action.phase,
          pot: action.pot,
          currentBet: action.currentBet,
          bigBlind: action.bigBlind,
          currentPlayer: action.currentPlayer,
          actedThisRound: action.actedThisRound || [],
          communityCards: community,
          holeCards: holeCards,
        };

        addLog(`Game state initialized. Phase: ${action.phase}, Pot: ${action.pot}`);
        break;
      }
      case 'fold':
      case 'check':
      case 'call':
      case 'raise':
      case 'allIn': {
        if (!gameState) return;
        const state = { ...gameState };
        state.players = { ...state.players };
        state.actedThisRound = [...(state.actedThisRound || [])];
        applyAction(state, fromPlayerId, action);
        gameState = state;
        break;
      }
    }
  }

  function dealHoleCards() {
    const state = { ...gameState };
    const deck = [...state.deck];
    state.holeCards = {};
    let idx = 0;

    for (const pid of state.playerOrder) {
      state.holeCards[pid] = [
        { ...deck[idx], _raw: true },
        { ...deck[idx + 1], _raw: true },
      ];
      idx += 2;
    }

    // Set community cards
    state.communityCards = [
      { ...deck[idx] },
      { ...deck[idx + 1] },
      { ...deck[idx + 2] },
      { ...deck[idx + 3] },
      { ...deck[idx + 4] },
    ];
    state.deckIndex = idx + 5;

    // Post blinds
    const bb = state.bigBlind;
    const sb = Math.floor(bb / 2);
    state.players[state.playerOrder[0]].bet = sb;
    state.players[state.playerOrder[0]].chips -= sb;
    state.players[state.playerOrder[1]].bet = bb;
    state.players[state.playerOrder[1]].chips -= bb;
    state.pot = sb + bb;
    state.currentBet = bb;
    state.currentPlayer = state.playerOrder.length > 2 ? state.playerOrder[2] : state.playerOrder[0];

    decryptedHoleCards = state.holeCards;
    decryptedCommunityCards = state.communityCards;
    gameState = state;

    addLog(`Dealt hole cards. ${state.players[state.playerOrder[0]].name} posts small blind (${sb}), ${state.players[state.playerOrder[1]].name} posts big blind (${bb})`);
  }

  // ─── Actions (shared by local & remote) ───────────────────────────
  function applyAction(state, actorId, action) {
    const player = { ...state.players[actorId] };
    const acted = [...(state.actedThisRound || [])];

    switch (action.type) {
      case ACTIONS.FOLD: {
        player.folded = true;
        addLog(`${player.name} folds`);
        break;
      }
      case ACTIONS.CHECK: {
        addLog(`${player.name} checks`);
        break;
      }
      case ACTIONS.CALL: {
        const toCall = state.currentBet - player.bet;
        player.chips -= toCall;
        player.bet = state.currentBet;
        state.pot += toCall;
        addLog(`${player.name} calls ${toCall}`);
        break;
      }
      case ACTIONS.RAISE: {
        const amount = action.amount || state.bigBlind;
        const totalBet = state.currentBet + amount;
        const toPut = totalBet - player.bet;
        player.chips -= toPut;
        player.bet = totalBet;
        state.pot += toPut;
        state.currentBet = totalBet;
        state.players = { ...state.players, [actorId]: player };
        state.actedThisRound = [actorId];
        addLog(`${player.name} raises to ${totalBet}`);
        advanceTurn(state);
        checkRoundComplete(state);
        return;
      }
      case ACTIONS.ALL_IN: {
        const toPut = player.chips;
        player.bet += toPut;
        state.pot += toPut;
        player.chips = 0;
        state.currentBet = Math.max(state.currentBet, player.bet);
        addLog(`${player.name} goes ALL IN with ${toPut}`);
        break;
      }
    }

    state.players = { ...state.players, [actorId]: player };
    if (!acted.includes(actorId)) acted.push(actorId);
    state.actedThisRound = acted;

    advanceTurn(state);
    checkRoundComplete(state);
  }

  function handleAction(action) {
    if (!isOurTurn || !gameState) return;

    const state = { ...gameState };
    state.players = { ...state.players };
    state.actedThisRound = [...(state.actedThisRound || [])];

    applyAction(state, ourPlayerId, action);

    gameState = state;
    roomClient.sendAction(action);
  }

  function advanceTurn(state) {
    const order = state.playerOrder;
    const currentIdx = order.indexOf(state.currentPlayer);
    let nextIdx = (currentIdx + 1) % order.length;
    let attempts = 0;

    while (
      state.players[order[nextIdx]].folded &&
      attempts < order.length
    ) {
      nextIdx = (nextIdx + 1) % order.length;
      attempts++;
    }

    state.currentPlayer = order[nextIdx];
  }

  function checkRoundComplete(state) {
    const activePlayers = state.playerOrder.filter(
      id => !state.players[id].folded
    );

    // Check if all but one folded
    if (activePlayers.length === 1) {
      const winner = state.players[activePlayers[0]];
      winner.chips += state.pot;
      addLog(`${winner.name} wins ${state.pot}!`);
      resetHand();
      return;
    }

    // Check if all active players have matched the bet
    const allMatched = activePlayers.every(
      id => state.players[id].bet === state.currentBet
    );
    const actedArr = state.actedThisRound || [];
    const allActed = actedArr.length >= activePlayers.length;

    if (allMatched && allActed) {
      advancePhase(state);
    }
  }

  function advancePhase(state) {
    const phases = [GAME_PHASES.PREFLOP, GAME_PHASES.FLOP, GAME_PHASES.TURN, GAME_PHASES.RIVER, GAME_PHASES.SHOWDOWN];
    const idx = phases.indexOf(state.phase);

    if (idx >= phases.length - 1) {
      // At showdown already
      handleShowdown(state);
      return;
    }

    state.phase = phases[idx + 1];
    state.currentBet = 0;
    state.actedThisRound = [];

    // Reset bets
    for (const id of Object.keys(state.players)) {
      state.players[id].bet = 0;
    }

    // Set first active player
    const activePlayers = state.playerOrder.filter(
      id => !state.players[id].folded
    );
    state.currentPlayer = activePlayers[0];

    addLog(`--- ${state.phase} ---`);

    if (state.phase === GAME_PHASES.SHOWDOWN) {
      handleShowdown(state);
    }
  }

  function handleShowdown(state) {
    const activePlayers = state.playerOrder.filter(
      id => !state.players[id].folded
    );

    const hands = {};
    for (const pid of activePlayers) {
      const hole = decryptedHoleCards[pid] || [];
      const community = decryptedCommunityCards || [];
      const allCards = [...hole, ...community].filter(c => c && c.suit);
      hands[pid] = evaluateHand(allCards);
    }

    // Find winner
    let winner = activePlayers[0];
    for (let i = 1; i < activePlayers.length; i++) {
      if (compareHands(hands[activePlayers[i]], hands[winner]) > 0) {
        winner = activePlayers[i];
      }
    }

    state.players[winner].chips += state.pot;
    addLog(`--- SHOWDOWN ---`);
    for (const pid of activePlayers) {
      addLog(`${state.players[pid].name}: ${hands[pid].desc || '???'}`);
    }
    addLog(`${state.players[winner].name} wins ${state.pot} with ${hands[winner].name}!`);

    // Reset for next hand
    setTimeout(() => resetHand(), 3000);
  }

  function resetHand() {
    const savedPlayers = gameState?.players || {};
    const savedOrder = gameState?.playerOrder || [];

    gameState = null;
    decryptedHoleCards = {};
    decryptedCommunityCards = [];
    addLog('--- New Hand ---');

    const isDealer = ourPlayerId === savedOrder[0];
    if (isDealer && !spectating && savedOrder.length >= 2) {
      setTimeout(() => {
        startNewHand(savedPlayers, savedOrder);
      }, 1500);
    }
  }

  function startNewHand(savedPlayers, savedOrder) {
    const playersObj = {};
    for (const id of savedOrder) {
      const p = savedPlayers[id];
      playersObj[id] = {
        id: p.id,
        name: p.name,
        did: p.did || null,
        chips: p.chips ?? 1000,
        bet: 0,
        folded: false,
        seat: p.seat,
      };
    }

    const deck = createDeck();
    const shuffled = [...deck].sort(() => Math.random() - 0.5);

    gameState = {
      players: playersObj,
      playerOrder: savedOrder,
      deck: shuffled,
      deckIndex: 0,
      phase: GAME_PHASES.PREFLOP,
      pot: 0,
      currentBet: 2,
      bigBlind: 2,
      currentPlayer: savedOrder[0],
      actedThisRound: [],
    };

    dealHoleCards();
    broadcastGameState();
  }

  // ─── Leave ────────────────────────────────────────────────────────
  function leave() {
    roomClient.leave();
    onLeaveRoom();
  }
</script>

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
