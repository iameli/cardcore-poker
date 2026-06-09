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

  /**
   * Read a room deep link out of the path. We use a path-based form —
   * https://cardco.re/at://did:.../re.cardco.poker.table/tid — so a room is a
   * plain shareable URL. Some servers/clients collapse the `//` in `at://`, so
   * we normalize defensively.
   */
  function readRoomUriFromPath() {
    let p;
    try {
      p = decodeURIComponent(window.location.pathname);
    } catch {
      p = window.location.pathname;
    }
    if (!p.startsWith("/at:")) return null;
    p = p.slice(1).replace(/^at:\/+/, "at://");
    return p.startsWith("at://") ? p : null;
  }

  // After auth resolves, route to the room deep link if present, else the lobby.
  function routeAfterAuth(s) {
    session = s;
    const deep = readRoomUriFromPath();
    if (deep) {
      roomUri = deep;
      page = "roomLobby";
    } else {
      page = "lobby";
    }
  }

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
            // Drop the OAuth hash but keep the path (may be a room deep link).
            window.history.replaceState({}, "", window.location.pathname);
            routeAfterAuth(s);
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
            routeAfterAuth(s);
            return;
          }
        } catch (err) {
          console.warn("OAuth session restore failed:", err);
        }
        try {
          const s = await restoreDemoSession();
          if (s) {
            routeAfterAuth(s);
          }
        } catch (err) {
          console.warn("Demo session restore failed:", err);
        }
      })();
    }
  });

  function onSignIn(sess) {
    routeAfterAuth(sess);
  }

  function onCreateRoom(uri) {
    roomUri = uri;
    page = "roomLobby";
    window.history.pushState({}, "", `/${uri}`);
  }

  function onStartGame() {
    if (roomUri) {
      tableUri = roomUri;
      page = "game";
      roomUri = null;
      window.history.replaceState({}, "", "/");
    }
  }

  function onLeaveRoom() {
    roomUri = null;
    page = "lobby";
    window.history.replaceState({}, "", "/");
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
    window.history.replaceState({}, "", "/");
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
