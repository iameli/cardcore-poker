<script>
  import SignIn from "./components/SignIn.svelte";
  import Lobby from "./components/Lobby.svelte";
  import RoomLobby from "./components/RoomLobby.svelte";
  import GameRoom from "./components/GameRoom.svelte";
  import PasswordGate from "./components/PasswordGate.svelte";
  import { handleCallback, getStoredSession, signOut } from "./lib/atproto.js";
  import { restoreDemoSession, clearDemoSession } from "./lib/demo-pds.js";

  // Temporary soft-launch gate — remove before going live properly.
  let unlocked = $state(
    typeof localStorage !== "undefined" && localStorage.getItem("cardcore_unlocked") === "1",
  );

  let page = $state("signin");
  let session = $state(null);
  let tableUri = $state(null);
  let roomUri = $state(null);

  $effect(() => {
    if (!unlocked) return;

    // Check for OAuth callback (atcute uses fragment mode: #code=...&state=...&iss=...)
    const hash = window.location.hash;
    // Only handle callback if all required params are present
    const hasCode = hash.includes("code=");
    const hasState = hash.includes("state=");
    const hasIss = hash.includes("iss=");

    if (hasCode && hasState && hasIss) {
      handleCallback()
        .then((s) => {
          if (s) {
            session = s;
            page = "lobby";
            window.history.replaceState({}, "", window.location.pathname);
          }
        })
        .catch((err) => {
          console.warn("OAuth callback processing failed:", err);
          // Clear the hash and show signin
          window.history.replaceState({}, "", window.location.pathname);
          page = "signin";
        });
    } else if (hasCode || hasState) {
      // Partial callback params — clear them
      console.warn("Incomplete OAuth callback params in URL, clearing hash");
      window.history.replaceState({}, "", window.location.pathname);
      page = "signin";
    } else {
      // Try OAuth session first, then fall back to a stored demo session.
      (async () => {
        try {
          const s = await getStoredSession();
          if (s) {
            session = s;
            page = "lobby";
            return;
          }
        } catch (err) {
          console.warn("OAuth session restore failed:", err);
        }
        try {
          const s = await restoreDemoSession();
          if (s) {
            session = s;
            page = "lobby";
          }
        } catch (err) {
          console.warn("Demo session restore failed:", err);
        }
      })();
    }
  });

  function onSignIn(sess) {
    session = sess;
    page = "lobby";
  }

  function onCreateRoom(uri) {
    roomUri = uri;
    page = "roomLobby";
  }

  function onStartGame() {
    if (roomUri) {
      tableUri = roomUri;
      page = "game";
      roomUri = null;
    }
  }

  function onLeaveRoom() {
    roomUri = null;
    page = "lobby";
  }

  function onJoinTable(uri) {
    tableUri = uri;
    page = "game";
  }

  function onLeaveTable() {
    tableUri = null;
    page = "lobby";
  }

  function onSignOut() {
    if (session?.isDemo) clearDemoSession();
    else signOut();
    session = null;
    page = "signin";
    tableUri = null;
    roomUri = null;
  }
</script>

<div class="app">
  {#if !unlocked}
    <PasswordGate onUnlock={() => (unlocked = true)} />
  {:else if page === "signin"}
    <SignIn {onSignIn} />
  {:else if page === "lobby"}
    <Lobby {session} {onJoinTable} {onCreateRoom} {onSignOut} />
  {:else if page === "roomLobby"}
    <RoomLobby {session} uri={roomUri} {onStartGame} {onLeaveRoom} />
  {:else if page === "game"}
    <GameRoom {session} {tableUri} onLeaveRoom={onLeaveTable} />
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
  :global(h1),
  :global(h2),
  :global(h3) {
    font-family: "Lady Radical", "Coder's Crux", monospace;
  }
  .app {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
  }
</style>
