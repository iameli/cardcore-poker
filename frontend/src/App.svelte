<script>
  import SignIn from './components/SignIn.svelte';
  import Lobby from './components/Lobby.svelte';
  import GameRoom from './components/GameRoom.svelte';
  import { handleCallback, getStoredSession, signOut, getDemoIdentity } from './lib/atproto.js';

  let page = $state('signin');
  let session = $state(null);
  let roomId = $state(null);
  let spectating = $state(false);

  $effect(() => {
    // Check for OAuth callback (atcute uses fragment mode: #code=...&state=...&iss=...)
    const hash = window.location.hash;
    // Only handle callback if all required params are present
    const hasCode = hash.includes('code=');
    const hasState = hash.includes('state=');
    const hasIss = hash.includes('iss=');

    if (hasCode && hasState && hasIss) {
      handleCallback()
        .then((s) => {
          if (s) {
            session = s;
            page = 'lobby';
            window.history.replaceState({}, '', window.location.pathname);
          }
        })
        .catch((err) => {
          console.warn('OAuth callback processing failed:', err);
          // Clear the hash and show signin
          window.history.replaceState({}, '', window.location.pathname);
          page = 'signin';
        });
    } else if (hasCode || hasState) {
      // Partial callback params — clear them
      console.warn('Incomplete OAuth callback params in URL, clearing hash');
      window.history.replaceState({}, '', window.location.pathname);
      page = 'signin';
    } else {
      // Check existing session
      getStoredSession()
        .then((s) => {
          if (s) {
            session = s;
            page = 'lobby';
          }
        })
        .catch((err) => {
          console.warn('Session restore failed:', err);
          page = 'signin';
        });
    }
  });

  function onSignIn(sess) {
    session = sess;
    page = 'lobby';
  }

  function onJoinRoom(id) {
    roomId = id;
    spectating = false;
    page = 'game';
  }

  function onSpectateRoom(id) {
    roomId = id;
    spectating = true;
    page = 'game';
  }

  function onLeaveRoom() {
    roomId = null;
    spectating = false;
    page = 'lobby';
  }

  function onSignOut() {
    signOut();
    session = null;
    page = 'signin';
    roomId = null;
  }
</script>

<div class="app">
  {#if page === 'signin'}
    <SignIn {onSignIn} />
  {:else if page === 'lobby'}
    <Lobby {session} {onJoinRoom} {onSpectateRoom} {onSignOut} />
  {:else if page === 'game'}
    <GameRoom {session} {roomId} {spectating} {onLeaveRoom} />
  {/if}
</div>

<style>
  :global(html) {
    font-size: 20.8px;
  }
  :global(*) {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
  }
  :global(body) {
    font-family: "Coder's Crux", monospace, system-ui;
    background: #ffffff;
    color: #1a1a1a;
    min-height: 100vh;
    overflow-x: hidden;
    image-rendering: pixelated;
    image-rendering: crisp-edges;
  }
  :global(h1), :global(h2), :global(h3) {
    font-family: 'Lady Radical', "Coder's Crux", monospace;
  }
  .app {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
  }
</style>
