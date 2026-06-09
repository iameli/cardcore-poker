<script>
  import { onMount, onDestroy } from "svelte";
  import * as dagCbor from "@ipld/dag-cbor";
  import PokerTable from "./PokerTable.svelte";
  import ActionBar from "./ActionBar.svelte";
  import GameLog from "./GameLog.svelte";
  import { initWasm, parseCard } from "../lib/cardcore-wasm.js";
  import { PlayerSession, generateSeed } from "../lib/game-session.js";
  import { Publisher } from "../lib/transport.js";
  import { FirehoseSubscriber } from "../lib/firehose.js";
  import { LEXICONS } from "../lib/atproto-publisher.js";
  import { GAME_PHASES } from "../lib/poker-engine.js";
  import { pdsForDid, resolveHandles } from "../lib/atproto.js";

  let { session, tableUri, onLeaveRoom } = $props();

  // ─── State ────────────────────────────────────────────────────────
  let logEvents = $state([]);
  let error = $state("");
  let tableRecord = $state(null);
  let playerDids = $state([]);
  let handleByDid = $state({});
  let ourPlayerIndex = $state(-1);
  let phase = $state("Init");
  let pot = $state(0);
  let chipsByDid = $state({});
  let betsByDid = $state({});
  let foldedByDid = $state({});
  let actionOnDid = $state(null);
  let holeCards = $state([]);
  let communityCards = $state([]);
  // Opponents' hole cards revealed at showdown (did → parsed cards), shown on
  // the table during the between-hands pause. Cleared when the next hand deals.
  let revealedByDid = $state({});
  let availableActions = $state([]);
  let raiseContext = $state(null);
  let isOurTurn = $state(false);
  let copied = $state(false);
  let gameOver = $state(false);

  // Pause between hands so players can read the showdown result before the
  // next hand is dealt.
  const NEXT_HAND_DELAY = 4000;

  let _publisher = null;
  let _session = null;
  let _firehose = null;
  let _tableTid = null;
  let _tableCid = null;
  let _loggedHandIndex = -1;
  let _advanceTimer = null;

  function addLog(msg) {
    logEvents = [...logEvents, msg];
    if (logEvents.length > 80) logEvents = logEvents.slice(-80);
  }

  function tidFromUri(uri) {
    return uri.split("/").pop();
  }

  /**
   * Human-readable label for an action CBOR payload, so the log shows every
   * protocol step — including the noninteractive ones (commitSeed, shuffle,
   * lock, deal reveals). Seeing those tick by is how you know the game is
   * working while nobody's betting.
   */
  function actionLabel(cbor) {
    try {
      const rec = dagCbor.decode(cbor);
      const kind = (rec.$type || "").split("#").pop() || "action";
      if (kind === "bet") {
        return rec.amount != null ? `${rec.action} ${rec.amount}` : rec.action;
      }
      if (kind === "revealLockKey") return `revealLockKey #${rec.deckPosition}`;
      return kind;
    } catch {
      return "action";
    }
  }

  // ─── Mount: fetch table, start session + poller ───────────────────
  onMount(async () => {
    if (!session?.client) {
      error = "No active session";
      return;
    }
    try {
      await initWasm();
      addLog("Fetching table…");
      const { record, cid } = await fetchTableRecord(tableUri);
      tableRecord = record;
      _tableCid = cid;
      _tableTid = tidFromUri(tableUri);
      playerDids = record.players;
      ourPlayerIndex = playerDids.indexOf(session.did);
      if (ourPlayerIndex < 0) {
        error = "You're not a player at this table";
        return;
      }
      addLog(
        `Table loaded — ${playerDids.length} players, ${record.startingChips} chips, ${record.smallBlind} SB`,
      );

      // Init chips
      const chips = {};
      for (const did of playerDids) chips[did] = record.startingChips;
      chipsByDid = chips;

      // Resolve handles in the background — DIDs are only the fallback.
      resolveHandles(playerDids, session.pdsUri)
        .then((m) => {
          handleByDid = Object.fromEntries(m);
        })
        .catch(() => {});

      const seed = restoreOrCreateSeed(tableUri);
      _publisher = new Publisher({ client: session.client, did: session.did });

      _session = new PlayerSession({
        did: session.did,
        seed,
        publishAction: async ({ seq, cbor }) => {
          addLog(`You: ${actionLabel(cbor)}`);
          await _publisher.publishAction({
            tableRef: { uri: tableUri, cid: _tableCid },
            seq,
            tableTid: _tableTid,
            actionCbor: cbor,
          });
        },
        onUpdate: refreshUi,
      });

      // Feed the table to our local agent first — that moves it out of Init
      // and into the CommitSeeds phase, so the firehose backfill (which may
      // include peer CommitSeeds already on disk) won't be rejected as
      // out-of-phase. getRecord strips $type; add it back for the lexicon.
      const tableForCbor = { $type: LEXICONS.TABLE, ...record };
      const tableCbor = dagCbor.encode(tableForCbor);
      addLog("Joining table…");
      await _session.receiveTable(tableCbor);

      const peerDids = playerDids.filter((d) => d !== session.did);
      _firehose = new FirehoseSubscriber({
        peerDids,
        tableUri,
        ownPdsUri: session.pdsUri,
        onAction: async (did, seq, cbor) => {
          addLog(`${nameFor(did)}: ${actionLabel(cbor)}`);
          try {
            await _session.receiveAction(cbor);
          } catch (e) {
            console.warn(`receiveAction(${did}@${seq}) failed:`, e?.message || e);
          }
        },
      });
      await _firehose.start();
      addLog("Subscribed to peer firehose — protocol running");
    } catch (e) {
      error = e?.message || String(e);
      console.error(e);
    }
  });

  onDestroy(() => {
    if (_advanceTimer) clearTimeout(_advanceTimer);
    _firehose?.stop();
    _session?.destroy();
  });

  // ─── Helpers ──────────────────────────────────────────────────────

  async function fetchTableRecord(uri) {
    const m = uri.match(/^at:\/\/([^/]+)\/([^/]+)\/(.+)$/);
    if (!m) throw new Error(`Bad table URI: ${uri}`);
    const [, repo, collection, rkey] = m;
    if (collection !== LEXICONS.TABLE) {
      throw new Error(`URI is not a poker table: ${collection}`);
    }
    // Records live on the AUTHOR's PDS, not ours. Resolve the repo DID to
    // its PDS endpoint and do an unauthenticated fetch — getRecord is public.
    const pds = await pdsForDid(repo, session.pdsUri);
    const url =
      `${pds}/xrpc/com.atproto.repo.getRecord` +
      `?repo=${encodeURIComponent(repo)}` +
      `&collection=${encodeURIComponent(collection)}` +
      `&rkey=${encodeURIComponent(rkey)}`;
    const res = await fetch(url);
    if (!res.ok) {
      const body = await res.text().catch(() => "");
      throw new Error(`getRecord(${repo}) ${res.status}: ${body.slice(0, 200)}`);
    }
    const data = await res.json();
    return { record: data.value, cid: data.cid };
  }

  function restoreOrCreateSeed(uri) {
    const key = `cardcore_seed:${uri}`;
    const stored = localStorage.getItem(key);
    if (stored) {
      const arr = stored.split(",").map(Number);
      if (arr.length === 32) return new Uint8Array(arr);
    }
    const seed = generateSeed();
    localStorage.setItem(key, Array.from(seed).join(","));
    return seed;
  }

  function refreshUi() {
    if (!_session) return;
    const gs = _session.gameState;
    if (gs) {
      pot = gs.pot ?? pot;
      const chips = {};
      const bets = {};
      const folded = {};
      for (const p of gs.players || []) {
        const did = playerDids[p.seat];
        if (did) {
          chips[did] = p.chips;
          bets[did] = p.bet;
          folded[did] = p.folded;
        }
      }
      chipsByDid = chips;
      betsByDid = bets;
      foldedByDid = folded;
      if (gs.actionOn != null) actionOnDid = playerDids[gs.actionOn];
    }
    holeCards = _session.holeCards;
    communityCards = _session.communityCards;
    phase = _session.phase;

    const commLen = communityCards.length;
    let uiPhase = "preflop";
    if (phase === "Showdown" || phase === "Complete") uiPhase = "showdown";
    else if (commLen >= 5) uiPhase = "river";
    else if (commLen >= 4) uiPhase = "turn";
    else if (commLen >= 3) uiPhase = "flop";

    if (_session.isComplete) {
      handleHandComplete();
    }

    if (_session.needsBet) {
      isOurTurn = true;
      availableActions = mapBetOptions(_session.betOptions);
      const ourChips = chipsByDid[session.did] ?? 0;
      const minRaise = tableRecord?.smallBlind ? tableRecord.smallBlind * 2 : 2;
      raiseContext = {
        min: minRaise,
        max: ourChips,
        pot,
        quickAmounts: buildQuickAmounts(pot, minRaise, ourChips),
      };
    } else {
      isOurTurn = false;
      availableActions = [];
      raiseContext = null;
    }
  }

  // Hand finished: log the result once, then either declare the game over or
  // schedule the next hand to start automatically.
  function handleHandComplete() {
    const result = _session.lastHandResult;
    if (result && result.hand_index > _loggedHandIndex) {
      _loggedHandIndex = result.hand_index;
      logHandResult(result);
      // Lay everyone's revealed hole cards face-up on the table for the
      // between-hands pause — the log alone is too easy to miss.
      const revealed = {};
      for (const s of result.shown || []) {
        const did = playerDids[s.seat];
        if (did) revealed[did] = s.cards.map(parseCard).filter(Boolean);
      }
      revealedByDid = revealed;
    }

    if (_session.gameOver) {
      if (!gameOver) {
        gameOver = true;
        const winnerDid = playerDids.find((d) => (chipsByDid[d] ?? 0) > 0);
        addLog(`🏆 Game over — ${nameFor(winnerDid)} wins!`);
      }
      return;
    }

    // Auto-advance to the next hand after a readable pause.
    if (!_advanceTimer) {
      _advanceTimer = setTimeout(advanceHand, NEXT_HAND_DELAY);
    }
  }

  function logHandResult(result) {
    addLog(`— Hand ${result.hand_index + 1} results —`);
    if (!result.by_fold) {
      for (const s of result.shown || []) {
        addLog(`  ${nameFor(playerDids[s.seat])}: ${s.cards.join(" ")} — ${s.hand_desc}`);
      }
    }
    for (const pot of result.pots || []) {
      const names = pot.winners.map((w) => nameFor(playerDids[w])).join(", ");
      if (!names) continue;
      if (result.by_fold) {
        addLog(`  ${names} wins ${pot.amount} (all others folded)`);
      } else {
        addLog(`  ${names} wins ${pot.amount}${pot.hand_desc ? ` with ${pot.hand_desc}` : ""}`);
      }
    }
  }

  async function advanceHand() {
    _advanceTimer = null;
    if (!_session || _session.gameOver) return;
    revealedByDid = {};
    addLog("Next hand…");
    try {
      await _session.nextHand();
    } catch (e) {
      console.warn("nextHand failed:", e?.message || e);
    }
  }

  function buildQuickAmounts(pot, min, max) {
    const out = [];
    const candidates = [
      ["1/3 POT", Math.floor(pot / 3)],
      ["1/2 POT", Math.floor(pot / 2)],
      ["POT", pot],
    ];
    for (const [label, amt] of candidates) {
      if (amt > min && amt <= max) out.push({ label, amount: amt });
    }
    return out;
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

  function shortDid(did) {
    return did?.slice(0, 12) + "…" + did?.slice(-6);
  }

  /** Display name for a player: handle when resolved, short DID as fallback. */
  function nameFor(did) {
    return handleByDid[did] || shortDid(did);
  }

  // ─── User actions ─────────────────────────────────────────────────

  async function handleAction(action) {
    if (!_session) return;
    let bet;
    if (action.type === "raise") bet = `raise:${action.amount || 2}`;
    else bet = action.type;
    // No addLog here — the bet is logged like every other action when its
    // record is published.
    try {
      await _session.bet(bet);
    } catch (e) {
      error = "Bet failed: " + (e?.message || e);
    }
  }

  async function copyTableUri() {
    try {
      await navigator.clipboard.writeText(tableUri);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {}
  }

  function leave() {
    onLeaveRoom();
  }

  // ─── Derived for PokerTable ───────────────────────────────────────
  const playerMap = $derived.by(() => {
    const m = {};
    for (let i = 0; i < playerDids.length; i++) {
      const did = playerDids[i];
      m[did] = {
        id: did,
        name: nameFor(did),
        did,
        chips: chipsByDid[did] ?? 0,
        bet: betsByDid[did] ?? 0,
        folded: !!foldedByDid[did],
        seat: i,
      };
    }
    return m;
  });

  const decryptedHoleCards = $derived({ ...revealedByDid, [session?.did]: holeCards });

  const playerDidsMap = $derived.by(() => {
    const m = {};
    for (const did of playerDids) m[did] = did;
    return m;
  });

  const gamePhase = $derived(
    phase === "Showdown" || phase === "Complete"
      ? "showdown"
      : communityCards.length >= 5
        ? "river"
        : communityCards.length >= 4
          ? "turn"
          : communityCards.length >= 3
            ? "flop"
            : "preflop",
  );
</script>

<div class="game-room">
  <header>
    <span class="handle-name">{session?.handle || shortDid(session?.did)}</span>
    <button
      class="room-id"
      onclick={copyTableUri}
      title="Click to copy table URI"
      data-testid="copy-table-uri"
    >
      table: <code>{tableUri ? tableUri.split("/").pop() : "…"}</code>
      <span class="copy-hint">{copied ? "✓ copied" : "copy"}</span>
    </button>
    <span class="phase-label" data-testid="phase">{gamePhase}</span>
    <button class="btn leave" onclick={leave}>Leave</button>
  </header>

  {#if error}
    <div class="error-banner">{error}</div>
  {/if}

  {#if gameOver}
    <div class="gameover-banner" data-testid="game-over">🏆 Game over — winner takes all</div>
  {/if}

  <div class="main-area">
    {#if !tableRecord}
      <p class="loading">Loading table…</p>
    {:else}
      <div class="game-layout">
        <div class="table-wrapper">
          <PokerTable
            players={playerMap}
            playerOrder={playerDids}
            playerDids={playerDidsMap}
            handleMap={handleByDid}
            holeCards={decryptedHoleCards}
            {communityCards}
            {pot}
            currentPlayer={actionOnDid}
            ourPlayerId={session?.did}
            {gamePhase}
            showAllCards={gamePhase === "showdown"}
          />
        </div>

        <div class="bottom-panel">
          <ActionBar
            actions={availableActions}
            raise={raiseContext}
            onAction={handleAction}
            {isOurTurn}
          />
          {#if !isOurTurn && actionOnDid && actionOnDid !== session?.did}
            <div class="waiting-turn">Waiting for {nameFor(actionOnDid)} to act…</div>
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
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1.5rem;
    background: #ffffff;
    border-bottom: 3px solid #1a1a1a;
    flex-wrap: wrap;
  }
  .handle-name {
    font-size: 0.5rem;
    color: #1a1a1a;
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
    font-size: 0.4rem;
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
  .phase-label {
    font-size: 0.4rem;
    color: #c0392b;
    letter-spacing: 2px;
    margin-left: auto;
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
  .leave:hover {
    background: #c0392b;
    color: #ffffff;
  }
  .error-banner {
    background: #c0392b;
    color: #ffffff;
    padding: 0.5rem;
    text-align: center;
    font-size: 0.45rem;
  }
  .gameover-banner {
    background: #1a7a3a;
    color: #ffffff;
    padding: 0.6rem;
    text-align: center;
    font-size: 0.55rem;
    letter-spacing: 1px;
  }
  .loading {
    text-align: center;
    padding: 2rem;
    font-size: 0.5rem;
    color: #1a1a1a;
    opacity: 0.6;
  }
  .main-area {
    flex: 1;
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
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
</style>
