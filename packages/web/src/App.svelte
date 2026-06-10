<script>
  import SignIn from "./components/SignIn.svelte";
  import Lobby from "./components/Lobby.svelte";
  import RoomLobby from "./components/RoomLobby.svelte";
  import GameRoom from "./components/GameRoom.svelte";
  import { handleCallback, getStoredSession, signOut } from "./lib/atproto.js";
  import { restoreDemoSession, clearDemoSession } from "./lib/demo-pds.js";
  import { fetchTableRecord, AUTH_EXPIRED_EVENT } from "./lib/transport.js";

  // Start in "loading" — we don't yet know whether a session will restore.
  // Showing SignIn immediately makes the login form flash on every page load
  // for signed-in users; every auth path below resolves to a real page.
  let page = $state("loading");
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

  // When the PDS rejects our credentials mid-flight (expired OAuth token),
  // remember where the user was, drop them at sign-in, and let routeAfterAuth
  // take them back after they re-authenticate.
  $effect(() => {
    const onExpired = () => {
      if (page === "signin") return;
      try {
        sessionStorage.setItem("cardcore_return_to", window.location.pathname);
      } catch {}
      // Purge the broken OAuth session so it isn't restored as-is. Demo
      // sessions stay — re-entering demo mode reuses the same account.
      if (session && !session.isDemo) signOut().catch(() => {});
      session = null;
      tableUri = null;
      roomUri = null;
      page = "signin";
    };
    window.addEventListener(AUTH_EXPIRED_EVENT, onExpired);
    return () => window.removeEventListener(AUTH_EXPIRED_EVENT, onExpired);
  });

  // After auth resolves, route to the table deep link if present, else the
  // lobby. A deep-linked table routes by its record: a published roster (2+
  // players, or startedAt set) means the game is live — enter the GameRoom,
  // whether we're a player in it or not. A lone-host record is still an open
  // room — go to the join lobby.
  function routeAfterAuth(s) {
    session = s;
    let deep = readRoomUriFromPath();
    // Coming back from a forced re-auth: the OAuth redirect may have landed
    // us on the redirect path instead of where the user was — restore it.
    try {
      const returnTo = sessionStorage.getItem("cardcore_return_to");
      if (returnTo) {
        sessionStorage.removeItem("cardcore_return_to");
        if (!deep) {
          window.history.replaceState({}, "", returnTo);
          deep = readRoomUriFromPath();
        }
      }
    } catch {}
    if (!deep) {
      page = "lobby";
      return;
    }
    (async () => {
      try {
        const { record } = await fetchTableRecord(deep, s.pdsUri);
        if ((record.players?.length ?? 0) >= 2 || record.startedAt) {
          tableUri = deep;
          page = "game";
          return;
        }
      } catch (e) {
        console.warn("Table lookup for deep link failed:", e?.message || e);
      }
      roomUri = deep;
      page = "roomLobby";
    })();
  }

  $effect(() => {
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
        // Demo sessions are a dev-only convenience — never restore one in prod.
        if (import.meta.env.DEV) {
          try {
            const s = await restoreDemoSession();
            if (s) {
              routeAfterAuth(s);
              return;
            }
          } catch (err) {
            console.warn("Demo session restore failed:", err);
          }
        }
        // No session anywhere — now we know it's time to sign in.
        page = "signin";
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
      // The table URL stays for the whole game — it's the shareable,
      // reload-safe address of the game itself.
    }
  }

  function onLeaveRoom() {
    roomUri = null;
    page = "lobby";
    window.history.replaceState({}, "", "/");
  }

  function onLeaveTable() {
    tableUri = null;
    page = "lobby";
    window.history.replaceState({}, "", "/");
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
  {#if page === "loading"}
    <div class="boot-loading" data-testid="loading">
      <div class="boot-card">
        <span class="boot-spinner"></span>
        Loading…
      </div>
    </div>
  {:else if page === "signin"}
    <SignIn {onSignIn} />
  {:else if page === "lobby"}
    <Lobby {session} {onCreateRoom} {onSignOut} />
  {:else if page === "roomLobby"}
    <RoomLobby {session} uri={roomUri} {onStartGame} {onLeaveRoom} />
  {:else if page === "game"}
    <GameRoom {session} {tableUri} onLeaveRoom={onLeaveTable} />
  {/if}
</div>

<style>
  :global(html) {
    font-size: 28px;
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
  .boot-loading {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .boot-card {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 0.5rem;
    color: #1a1a1a;
    border: 3px solid #1a1a1a;
    box-shadow: 6px 6px 0 #1a1a1a;
    padding: 0.8rem 1.2rem;
    letter-spacing: 1px;
  }
  .boot-spinner {
    width: 0.6rem;
    height: 0.6rem;
    border: 3px solid #1a1a1a;
    border-top-color: #c0392b;
    animation: boot-spin 0.8s steps(8) infinite;
  }
  @keyframes boot-spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
