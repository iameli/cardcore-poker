<script>
  import { onMount, onDestroy } from "svelte";
  import { Publisher, fetchTableRecord } from "../lib/transport.js";
  import { JoinRequestWatcher } from "../lib/discovery.js";
  import { resolveDidToHandle } from "../lib/atproto.js";

  let { session, uri, onStartGame, onLeaveRoom } = $props();

  // The room URI is at://<host>/re.cardco.poker.table/<tid>. We're the host
  // iff the repo segment is our own DID.
  const repo = $derived(uri ? uri.split("/")[2] : "");
  const tid = $derived(uri ? uri.split("/").pop() : "");
  const isHost = $derived(!!session?.did && repo === session.did);
  const roomLink = $derived(uri ? `${window.location.origin}/${uri}` : "");

  let copied = $state(false);
  let error = $state("");
  let tableInfo = $state(null); // { startingChips, smallBlind, players }

  // host-side state
  let pending = $state([]); // [{ joinerDid }] awaiting approval
  let approved = $state([]); // [joinerDid] admitted to the roster
  let starting = $state(false);

  // joiner-side state
  let requested = $state(false);
  let accepted = $state(false);

  let _publisher = null;
  let _watcher = null;
  let _poll = null;

  let handleByDid = $state({});

  function shortDid(did) {
    return did ? did.slice(0, 10) + "…" + did.slice(-4) : "";
  }

  /** Display name for a player: handle when resolved, short DID as fallback. */
  function nameFor(did) {
    return handleByDid[did] || shortDid(did);
  }

  /** Resolve a DID's handle in the background and cache it for display. */
  function learnHandle(did) {
    if (!did || handleByDid[did]) return;
    resolveDidToHandle(did, session?.pdsUri)
      .then((handle) => {
        if (handle) handleByDid = { ...handleByDid, [did]: handle };
      })
      .catch(() => {});
  }

  onMount(async () => {
    if (!session?.client || !uri) return;
    _publisher = new Publisher({ client: session.client, did: session.did });
    learnHandle(repo);
    learnHandle(session.did);

    try {
      const { record } = await fetchTableRecord(uri, session.pdsUri);
      tableInfo = {
        startingChips: record.startingChips,
        smallBlind: record.smallBlind,
        players: record.players || [],
      };
    } catch (e) {
      // Brand-new host record may not be readable for a beat; fall back to the
      // protocol defaults so the host UI still renders.
      tableInfo = { startingChips: 1000, smallBlind: 10, players: [session.did] };
    }

    if (isHost) {
      // Discover join requests (table records other repos publish at our rkey).
      _watcher = new JoinRequestWatcher({
        hostDid: session.did,
        tableRkey: tid,
        ownPdsUri: session.pdsUri,
        onRequest: ({ joinerDid }) => {
          if (joinerDid === session.did) return;
          if (approved.includes(joinerDid)) return;
          if (pending.some((p) => p.joinerDid === joinerDid)) return;
          learnHandle(joinerDid);
          pending = [...pending, { joinerDid }];
        },
      });
      _watcher.start();
    } else {
      // Everyone else watches the host record for the game starting — whether
      // they've requested a seat or not. When it starts, GameRoom decides if
      // they're a player (on the roster) or a spectator.
      _poll = setInterval(pollAcceptance, 1500);
    }
  });

  onDestroy(() => {
    _watcher?.stop();
    if (_poll) clearInterval(_poll);
  });

  // ─── Host actions ─────────────────────────────────────────────────
  function approve(joinerDid) {
    if (!approved.includes(joinerDid)) approved = [...approved, joinerDid];
    pending = pending.filter((p) => p.joinerDid !== joinerDid);
  }

  async function startGame() {
    if (!isHost || starting) return;
    if (approved.length < 1) {
      error = "Approve at least one player before starting";
      return;
    }
    starting = true;
    error = "";
    try {
      // Publish the locked roster + startedAt. This host record is the single
      // source of truth every player references for the hand (seat order, CID).
      const roster = [session.did, ...approved];
      await _publisher.publishTableWithRkey(tid, {
        players: roster,
        startingChips: tableInfo?.startingChips ?? 1000,
        smallBlind: tableInfo?.smallBlind ?? 10,
        startedAt: new Date().toISOString(),
      });
      onStartGame();
    } catch (e) {
      error = e?.message || String(e);
      starting = false;
    }
  }

  // ─── Joiner actions ───────────────────────────────────────────────
  async function requestJoin() {
    if (isHost || requested) return;
    error = "";
    try {
      const { record } = await fetchTableRecord(uri, session.pdsUri);
      const players = record.players || [];
      const roster = players.includes(session.did) ? players : [...players, session.did];
      // Publish our "suggested addition": the host's table at the same rkey on
      // OUR repo, with us appended. The host discovers this on the firehose.
      await _publisher.publishTableWithRkey(tid, {
        players: roster,
        startingChips: record.startingChips,
        smallBlind: record.smallBlind,
      });
      requested = true;
      pollAcceptance();
    } catch (e) {
      error = e?.message || String(e);
    }
  }

  async function pollAcceptance() {
    try {
      const { record } = await fetchTableRecord(uri, session.pdsUri);
      const players = record.players || [];
      accepted = players.includes(session.did);
      if (record.startedAt) {
        // Game's on. Enter it either way — as a player if the host put us on
        // the roster, as a spectator if not.
        if (_poll) clearInterval(_poll);
        _poll = null;
        onStartGame();
      }
    } catch {
      /* transient; keep polling */
    }
  }

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(roomLink);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {}
  }

  const playerName = $derived(session?.handle || session?.name || "Player");
  const rosterCount = $derived(1 + approved.length);
</script>

<div class="room-lobby">
  <header>
    <div class="user-info">
      <span class="name">{playerName}</span>
      <span class="did" title={session?.did}>
        {session?.did?.slice(0, 12)}…{session?.did?.slice(-6)}
      </span>
    </div>
    <button class="btn logout" onclick={onLeaveRoom} data-testid="leave-room">Leave</button>
  </header>

  <div class="content">
    {#if isHost}
      <h2>Your Open Room</h2>

      <section class="card">
        <h3>Share This Link</h3>
        <p class="hint">Send this to players you want to invite:</p>
        <div class="uri-container" data-testid="copy-table-uri">
          <code>{tid}</code>
          <button class="btn secondary" onclick={copyLink} data-testid="copy-uri-button">
            {copied ? "Copied!" : "Copy"}
          </button>
        </div>
      </section>

      <section class="card">
        <h3>Join Requests</h3>
        {#if pending.length === 0}
          <p class="hint" data-testid="no-requests">Waiting for players to request to join…</p>
        {:else}
          <ul class="request-list">
            {#each pending as req (req.joinerDid)}
              <li class="request-row">
                <span class="req-did" title={req.joinerDid}>{nameFor(req.joinerDid)}</span>
                <button
                  class="btn primary small"
                  onclick={() => approve(req.joinerDid)}
                  data-testid="approve-request"
                >
                  Approve
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="card">
        <h3>Roster ({rosterCount})</h3>
        <ul class="roster-list" data-testid="roster">
          <li class="roster-row">
            <span class="req-did">{session?.handle || nameFor(session?.did)}</span>
            <span class="tag">host</span>
          </li>
          {#each approved as did (did)}
            <li class="roster-row">
              <span class="req-did" title={did}>{nameFor(did)}</span>
              <span class="tag ok">approved</span>
            </li>
          {/each}
        </ul>
      </section>

      <div class="actions">
        <button
          class="btn primary"
          onclick={startGame}
          disabled={starting || approved.length < 1}
          data-testid="start-game"
        >
          {starting ? "Starting…" : "Start Game"}
        </button>
      </div>
    {:else}
      <h2>Join Room</h2>

      <section class="card">
        <h3>Table</h3>
        <p class="hint">Hosted by {nameFor(repo)}</p>
        {#if tableInfo}
          <p class="hint">
            {tableInfo.startingChips} chips · {tableInfo.smallBlind} small blind
          </p>
        {/if}
      </section>

      <div class="actions">
        {#if !requested}
          <button class="btn primary" onclick={requestJoin} data-testid="request-join">
            Request to Join
          </button>
        {:else if accepted}
          <p class="status ok" data-testid="join-status">
            Approved — waiting for the host to start…
          </p>
        {:else}
          <p class="status" data-testid="join-status">Requested — waiting for the host…</p>
        {/if}
      </div>
    {/if}

    {#if error}
      <p class="error" data-testid="room-error">{error}</p>
    {/if}
  </div>
</div>

<style>
  .room-lobby {
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
    font-size: 0.4rem;
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
    margin-bottom: 0.5rem;
  }
  .uri-container {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  code {
    flex: 1;
    padding: 0.7rem;
    background: #f5f5f5;
    border: 2px solid #1a1a1a;
    font-family: monospace;
    font-size: 0.42rem;
    color: #1a1a1a;
    word-break: break-all;
    overflow-wrap: break-word;
  }
  .request-list,
  .roster-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .request-row,
  .roster-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    border: 2px solid #1a1a1a;
    padding: 0.4rem 0.6rem;
  }
  .req-did {
    font-size: 0.4rem;
    color: #1a1a1a;
  }
  .tag {
    font-size: 0.4rem;
    letter-spacing: 1px;
    opacity: 0.6;
    text-transform: uppercase;
  }
  .tag.ok {
    color: #1a7a3a;
    opacity: 1;
  }
  .status {
    font-size: 0.45rem;
    text-align: center;
    color: #1a1a1a;
    opacity: 0.7;
  }
  .status.ok {
    color: #1a7a3a;
    opacity: 1;
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
  .btn.small {
    padding: 0.4rem 0.8rem;
    font-size: 0.4rem;
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
    white-space: nowrap;
    flex-shrink: 0;
  }
  .logout {
    font-size: 0.4rem;
    padding: 0.4rem 0.8rem;
  }
  .logout:hover {
    background: #c0392b;
    color: #ffffff;
  }
  .actions {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    margin-top: 1rem;
  }
  .error {
    color: #c0392b;
    font-size: 0.45rem;
    text-align: center;
    padding: 0.5rem;
    border: 2px dashed #c0392b;
  }
</style>
