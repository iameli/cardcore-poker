<script>
  import { initWasm } from "../lib/cardcore-wasm.js";
  import { Publisher } from "../lib/transport.js";

  let { session, onJoinTable, onSignOut } = $props();

  let opponentHandle = $state("");
  let joinUri = $state("");
  let creating = $state(false);
  let error = $state("");

  $effect(() => {
    initWasm().catch(() => {});
  });

  /**
   * Resolve a handle to its DID. For demo accounts on the local PDS, the
   * Vite proxy forwards to com.atproto.identity.resolveHandle. For real
   * Bluesky handles in production, the same call works against the user's
   * PDS.
   */
  async function resolveHandle(handle) {
    const trimmed = handle.trim().replace(/^@/, "");
    const url = `/xrpc/com.atproto.identity.resolveHandle?handle=${encodeURIComponent(trimmed)}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`Could not resolve handle: ${trimmed}`);
    const data = await res.json();
    return data.did;
  }

  async function createTable() {
    if (!opponentHandle.trim()) {
      error = "Enter your opponent's handle";
      return;
    }
    if (!session?.client) {
      error = "Sign in first";
      return;
    }
    creating = true;
    error = "";
    try {
      const opponentDid = await resolveHandle(opponentHandle);
      if (opponentDid === session.did) {
        error = "Pick a different player";
        return;
      }
      const publisher = new Publisher({ client: session.client, did: session.did });
      const result = await publisher.createTable({
        players: [session.did, opponentDid],
        startingChips: 1000,
        smallBlind: 10,
      });
      onJoinTable(result.uri);
    } catch (e) {
      error = e?.message || String(e);
    } finally {
      creating = false;
    }
  }

  function joinTable() {
    const uri = joinUri.trim();
    if (!uri.startsWith("at://")) {
      error = "Paste an at:// URI";
      return;
    }
    onJoinTable(uri);
  }

  const playerName = $derived(session?.handle || session?.name || "Player");
</script>

<div class="lobby">
  <header>
    <div class="user-info">
      <span class="name">{playerName}</span>
      <span class="did" title={session?.did}>
        {session?.did?.slice(0, 12)}…{session?.did?.slice(-6)}
      </span>
    </div>
    <button class="btn logout" onclick={onSignOut}>Sign Out</button>
  </header>

  <div class="content">
    <h2>Lobby</h2>

    <section class="card">
      <h3>Start a New Table</h3>
      <p class="hint">Enter the handle of the player you want to play with.</p>
      <div class="join-row">
        <input
          type="text"
          placeholder="opponent.bsky.social"
          bind:value={opponentHandle}
          disabled={creating}
          data-testid="opponent-handle"
        />
        <button
          class="btn primary"
          onclick={createTable}
          disabled={creating}
          data-testid="create-table"
        >
          {creating ? "Creating…" : "Create Table"}
        </button>
      </div>
    </section>

    <div class="divider"><span>or</span></div>

    <section class="card">
      <h3>Join an Existing Table</h3>
      <p class="hint">Paste the table's AT URI (the creator shares it with you).</p>
      <div class="join-row">
        <input
          type="text"
          placeholder="at://did:plc:.../re.cardco.poker.table/..."
          bind:value={joinUri}
          onkeydown={(e) => e.key === "Enter" && joinTable()}
          data-testid="join-uri"
        />
        <button class="btn secondary" onclick={joinTable} data-testid="join-table">Join</button>
      </div>
    </section>

    {#if error}
      <p class="error">{error}</p>
    {/if}
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
    flex-direction: column;
    gap: 0.1rem;
  }
  .name {
    font-size: 0.5rem;
    color: #1a1a1a;
  }
  .did {
    font-size: 0.32rem;
    color: #1a1a1a;
    opacity: 0.5;
  }
  .content {
    flex: 1;
    max-width: 600px;
    width: 100%;
    margin: 0 auto;
    padding: 2rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  h2 {
    text-align: center;
    font-size: 1.7rem;
    margin-bottom: 1rem;
    color: #1a1a1a;
    letter-spacing: 2px;
  }
  h3 {
    font-size: 0.55rem;
    color: #1a1a1a;
    margin-bottom: 0.5rem;
  }
  .card {
    border: 3px solid #1a1a1a;
    box-shadow: 6px 6px 0 #1a1a1a;
    padding: 1rem;
    background: #ffffff;
  }
  .hint {
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.6;
    margin-bottom: 0.75rem;
  }
  .btn {
    padding: 0.7rem 1.2rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    font-family: inherit;
    font-size: 0.45rem;
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
  }
  .secondary {
    background: #1a1a1a;
    color: #ffffff;
  }
  .logout {
    font-size: 0.4rem;
    padding: 0.4rem 0.8rem;
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
    padding: 0.7rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    background: #ffffff;
    color: #1a1a1a;
    font-family: inherit;
    font-size: 0.42rem;
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
    content: "";
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
    padding: 0.5rem;
    border: 2px dashed #c0392b;
  }
</style>
