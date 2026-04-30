<script>
  import { onMount } from 'svelte';
  import { ATPublisher, buildTableRecord } from '../lib/atproto-publisher.js';
  import { initWasm } from '../lib/cardcore-wasm.js';
  import { fetchProfile } from '../lib/atproto.js';

  let { session, onJoinRoom, onSpectateRoom, onSignOut } = $props();

  let rooms = $state([]);
  let joinId = $state('');
  let creating = $state(false);
  let error = $state('');
  let loaded = $state(false);
  let atpRoomUri = $state(null);
  let userAvatar = $state(null);

  onMount(() => {
    if (session?.session) {
      const did = session.did;
      if (did) {
        fetchProfile(did).then(profile => {
          if (profile?.avatar) userAvatar = profile.avatar;
        });
      }
    }
  });

  $effect(() => {
    initWasm().catch(() => {});
    fetchRooms();
    const interval = setInterval(fetchRooms, 5000);
    return () => clearInterval(interval);
  });

  async function fetchRooms() {
    try {
      const res = await fetch('/api/rooms');
      if (res.ok) {
        rooms = await res.json();
        loaded = true;
      }
    } catch {
      // server may not be running yet
    }
  }

  async function createRoom() {
    creating = true;
    error = '';
    try {
      const res = await fetch('/api/rooms', { method: 'POST' });
      if (res.ok) {
        const { roomId } = await res.json();

        // If we have a real AT Protocol session, publish a table record
        if (session?.session?.fetchHandler && session?.did) {
          try {
            const publisher = new ATPublisher({
              handler: session.session.fetchHandler,
              did: session.did,
            });
            const result = await publisher.createTable({
              players: [session.did],
              startingChips: 1000,
              smallBlind: 10,
            });
            atpRoomUri = result.uri;
            console.log('[Lobby] Published table record:', result.uri);
            // Register the AT URI with the room server
            await fetch(`/api/rooms/${roomId}/atp`, {
              method: 'PUT',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ atpUri: result.uri }),
            });
          } catch (e) {
            console.warn('[Lobby] AT Protocol publish failed (non-fatal):', e.message);
          }
        }

        onJoinRoom(roomId);
      } else {
        error = 'Failed to create room';
      }
    } catch {
      error = 'Server not available. Start the server with: npm run dev:server';
    } finally {
      creating = false;
    }
  }

  function joinRoom() {
    const id = joinId.trim();
    if (!id) {
      error = 'Enter a room ID';
      return;
    }
    onJoinRoom(id);
  }

  const playerName = $derived(
    session?.handle || session?.name || 'Player'
  );
</script>

<div class="lobby">
  <header>
    <div class="user-info">
      {#if userAvatar}
        <div class="avatar-circle">
          <img src={userAvatar} alt="avatar" />
        </div>
      {/if}
      <span class="name">{playerName}</span>
    </div>
    <button class="btn logout" onclick={onSignOut}>Leave</button>
  </header>

  <div class="content">
    <h2>Lobby</h2>

    <div class="actions">
      <button class="btn primary" onclick={createRoom} disabled={creating}>
        {creating ? 'Creating...' : 'Create New Room'}
      </button>

      <div class="divider"><span>or join existing</span></div>

      <div class="join-row">
        <input
          type="text"
          placeholder="Room ID"
          bind:value={joinId}
          onkeydown={(e) => e.key === 'Enter' && joinRoom()}
        />
        <button class="btn secondary" onclick={joinRoom}>
          Join
        </button>
      </div>

      {#if error}
        <p class="error">{error}</p>
      {/if}
    </div>

    <div class="room-list">
      <h3>Active Rooms ({rooms.length})</h3>
      {#if !loaded}
        <p class="loading">Connecting to server...</p>
      {:else if rooms.length === 0}
        <p class="empty">No rooms yet. Create one!</p>
      {:else}
        {#each rooms as room}
          <div class="room-card">
            <div class="room-id">{room.id}</div>
            <div class="room-players">
              {room.playerCount} player{room.playerCount !== 1 ? 's' : ''}
              {#if room.spectatorCount > 0}
                <span class="spectator-count"> ({room.spectatorCount} watching)</span>
              {/if}
              {#if room.hasGame}
                <span class="live-badge">● LIVE</span>
              {/if}
            </div>
            <button class="btn small" onclick={() => onJoinRoom(room.id)}>
              Join
            </button>
            <button class="btn small spectate" onclick={() => onSpectateRoom(room.id)}>
              Watch
            </button>
          </div>
        {/each}
      {/if}
    </div>
  </div>
</div>

<style>
  .lobby {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    background: #ffffff;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1rem 1.5rem;
    background: #ffffff;
    border-bottom: 3px solid #1a1a1a;
  }
  .user-info {
    display: flex;
    align-items: center;
    gap: 0.5rem;
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
  .name {
    font-size: 0.5rem;
    color: #1a1a1a;
  }
  .content {
    flex: 1;
    max-width: 600px;
    width: 100%;
    margin: 0 auto;
    padding: 2rem 1rem;
  }
  h2 {
    text-align: center;
    font-size: 1.7rem;
    margin-bottom: 2rem;
    color: #1a1a1a;
    letter-spacing: 2px;
  }
  h3 {
    font-size: 0.5rem;
    color: #1a1a1a;
    margin-bottom: 0.75rem;
    opacity: 0.7;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    margin-bottom: 2rem;
  }
  .btn {
    padding: 0.75rem 1.5rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    font-family: inherit;
    font-size: 0.5rem;
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
  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
    transform: none;
    box-shadow: 3px 3px 0 #1a1a1a;
  }
  .primary {
    background: #c0392b;
    color: #ffffff;
    border-color: #1a1a1a;
  }
  .secondary {
    background: #1a1a1a;
    color: #ffffff;
    border-color: #1a1a1a;
    white-space: nowrap;
  }
  .small {
    padding: 0.4rem 0.8rem;
    font-size: 0.4rem;
    background: #1a1a1a;
    color: #ffffff;
  }
  .logout {
    background: #ffffff;
    color: #1a1a1a;
    border: 2px solid #1a1a1a;
    font-size: 0.4rem;
    padding: 0.4rem 0.8rem;
    box-shadow: 3px 3px 0 #1a1a1a;
  }
  .logout:hover {
    background: #c0392b;
    color: #ffffff;
  }
  .join-row {
    display: flex;
    gap: 0.5rem;
  }
  .join-row input {
    flex: 1;
    padding: 0.75rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    background: #ffffff;
    color: #1a1a1a;
    font-family: inherit;
    font-size: 0.5rem;
    outline: none;
  }
  .join-row input:focus {
    border-color: #c0392b;
    box-shadow: 3px 3px 0 #c0392b;
  }
  .divider {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .divider::before,
  .divider::after {
    content: '';
    flex: 1;
    height: 2px;
    background: #1a1a1a;
  }
  .divider span {
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.6;
  }
  .error {
    color: #c0392b;
    font-size: 0.45rem;
    text-align: center;
  }
  .room-list {
    background: #ffffff;
    border: 3px solid #1a1a1a;
    border-radius: 0;
    padding: 1rem;
  }
  .room-card {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 0;
    border-bottom: 2px solid #1a1a1a;
  }
  .room-card:last-child {
    border-bottom: none;
  }
  .room-id {
    font-size: 0.5rem;
    color: #1a1a1a;
    flex: 1;
  }
  .room-players {
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.6;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .spectator-count {
    color: #888;
  }
  .live-badge {
    color: #c0392b;
    animation: pulse 1.5s infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }
  .spectate {
    background: #ffffff;
    color: #1a1a1a;
    border: 2px solid #1a1a1a;
  }
  .spectate:hover {
    background: #f0f0f0;
  }
  .loading, .empty {
    font-size: 0.45rem;
    color: #1a1a1a;
    opacity: 0.5;
    text-align: center;
    padding: 1rem;
  }
</style>
