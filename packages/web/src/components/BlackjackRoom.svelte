<script>
  import { onMount, onDestroy } from "svelte";
  import * as dagCbor from "@ipld/dag-cbor";
  import BlackjackTable from "./BlackjackTable.svelte";
  import ActionBar from "./ActionBar.svelte";
  import GameLog from "./GameLog.svelte";
  import { initWasm, parseCard } from "../lib/cardcore-wasm.js";
  import { BlackjackSession } from "../lib/blackjack-session.js";
  import { generateSeed } from "../lib/game-session.js";
  import { GAMES } from "../lib/games.js";
  import { Publisher, fetchTableRecord } from "../lib/transport.js";
  import { FirehoseSubscriber } from "../lib/firehose.js";
  import { resolveHandles } from "../lib/atproto.js";

  let { session, tableUri, onLeaveRoom } = $props();

  const game = GAMES.blackjack;

  // ─── State ────────────────────────────────────────────────────────
  let logEvents = $state([]);
  let error = $state("");
  let tableRecord = $state(null);
  let playerDids = $state([]);
  let handleByDid = $state({});
  let phase = $state("Init");
  let gs = $state(null); // parsed game_state JSON from the agent
  let myNeed = $state(null); // { kind: "wager"|"insurance"|"decision", ... }
  let wagerAmount = $state(0);
  let copied = $state(false);
  let gameOver = $state(false);
  let winnerDid = $state(null);
  let isSpectator = $state(false);
  let roundOver = $state(false);
  let logOpen = $state(false);

  // ─── Scale-to-fit (same approach as GameRoom) ─────────────────────
  const DESIGN_W = 900;
  let fitBoxW = $state(0);
  let fitBoxH = $state(0);
  let fitContentH = $state(0);
  const fitScale = $derived.by(() => {
    if (!fitBoxW || !fitBoxH || !fitContentH) return 1;
    return Math.min(fitBoxW / DESIGN_W, fitBoxH / fitContentH, 2);
  });

  // Pause between rounds so players can read the results before the next
  // round is dealt.
  const NEXT_ROUND_DELAY = 4000;

  let _publisher = null;
  let _session = null;
  let _firehose = null;
  let _tableTid = null;
  let _tableCid = null;
  let _loggedRoundIndex = -1;
  let _advanceTimer = null;

  function addLog(msg) {
    logEvents = [...logEvents, msg];
    if (logEvents.length > 80) logEvents = logEvents.slice(-80);
  }

  function tidFromUri(uri) {
    return uri.split("/").pop();
  }

  /** Human-readable label for an action CBOR payload (for the log). */
  function actionLabel(cbor) {
    try {
      const rec = dagCbor.decode(cbor);
      const kind = (rec.$type || "").split("#").pop() || "action";
      if (kind === "wager") return `wager ${rec.amount}`;
      if (kind === "insurance") return rec.take ? "insurance: yes" : "insurance: no";
      if (kind === "decision") return rec.move;
      if (kind === "revealLockKey") return `revealLockKey #${rec.deckPosition}`;
      return kind;
    } catch {
      return "action";
    }
  }

  // ─── Mount: fetch table, start session + firehose ─────────────────
  onMount(async () => {
    if (!session?.client) {
      error = "No active session";
      return;
    }
    try {
      await initWasm();
      addLog("Fetching table…");
      const { record, cid } = await fetchTableRecord(tableUri, session.pdsUri);
      tableRecord = record;
      _tableCid = cid;
      _tableTid = tidFromUri(tableUri);
      playerDids = record.players;
      const ourPlayerIndex = playerDids.indexOf(session.did);
      isSpectator = ourPlayerIndex < 0;
      addLog(
        `Table loaded — ${playerDids.length} players, ${record.startingChips} chips, ${record.minBet} min bet`,
      );
      if (isSpectator) addLog("Spectating — replaying the game from PDS records…");

      resolveHandles(playerDids, session.pdsUri)
        .then((m) => {
          handleByDid = Object.fromEntries(m);
        })
        .catch(() => {});

      // Spectators get a throwaway seed (their agent never emits) and a
      // no-op publisher.
      const seed = isSpectator ? generateSeed() : restoreOrCreateSeed(tableUri);
      _publisher = new Publisher({
        client: session.client,
        did: session.did,
        tableCollection: game.tableCollection,
        actionCollection: game.actionCollection,
      });

      _session = new BlackjackSession({
        did: session.did,
        seed,
        publishAction: isSpectator
          ? async () => {}
          : async ({ seq, cbor }) => {
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

      // Feed the table to our local agent first (moves it out of Init so the
      // firehose backfill isn't rejected). getRecord strips $type; add it
      // back for the lexicon.
      const tableForCbor = { $type: game.tableCollection, ...record };
      const tableCbor = dagCbor.encode(tableForCbor);
      addLog(isSpectator ? "Watching table…" : "Joining table…");
      await _session.receiveTable(tableCbor);

      // Subscribe to EVERY player's repo — including our own, which is what
      // makes a page reload resumable (re-derivable actions dedupe; our past
      // wagers/decisions re-apply).
      _firehose = new FirehoseSubscriber({
        peerDids: playerDids,
        tableUri,
        ownPdsUri: session.pdsUri,
        actionCollection: game.actionCollection,
        onAction: async (did, seq, cbor) => {
          const fromSelf = did === session.did;
          if (!fromSelf) addLog(`${nameFor(did)}: ${actionLabel(cbor)}`);
          try {
            await _session.receiveAction(cbor, { fromSelf, seq });
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
    gs = _session.gameState;
    phase = _session.phase;
    myNeed = _session.need;
    if (myNeed?.kind === "wager") {
      // Default the input to the table minimum each time the panel opens.
      if (!wagerAmount || wagerAmount < myNeed.min || wagerAmount > myNeed.max) {
        wagerAmount = myNeed.min;
      }
    }
    roundOver = _session.isComplete;
    if (_session.isComplete) {
      handleRoundComplete();
    }
  }

  // Round finished: log the result once, then either declare the game over
  // or schedule the next round automatically.
  function handleRoundComplete() {
    const result = _session.lastRoundResult;
    if (result && result.round_index > _loggedRoundIndex) {
      _loggedRoundIndex = result.round_index;
      logRoundResult(result);
    }

    if (_session.gameOver) {
      if (!gameOver) {
        gameOver = true;
        const players = gs?.players || [];
        const winnerSeat = players.find((p) => p.chips > 0)?.seat;
        winnerDid = winnerSeat != null ? playerDids[winnerSeat] : null;
        addLog(`🏆 Game over — ${nameFor(winnerDid)} wins!`);
      }
      return;
    }

    // Auto-advance after a readable pause; replays (a backlog bigger than
    // the roster) skip the pause.
    if (!_advanceTimer) {
      const catchingUp = isSpectator
        ? _session.pendingCount > 0
        : _session.pendingCount > playerDids.length;
      _advanceTimer = setTimeout(advanceRound, catchingUp ? 250 : NEXT_ROUND_DELAY);
    }
  }

  function logRoundResult(result) {
    addLog(`— Round ${result.round_index + 1} results —`);
    const b = result.banker;
    const bankerLine = b.blackjack ? "blackjack" : b.bust ? `bust (${b.total})` : b.total;
    addLog(`  bank ${nameFor(playerDids[b.seat])}: ${b.cards.join(" ")} — ${bankerLine}`);
    for (const p of result.players || []) {
      const did = playerDids[p.seat];
      if (!did || p.seat === b.seat) continue;
      for (const h of p.hands || []) {
        const payout = h.payout > 0 ? ` (+${h.payout})` : "";
        addLog(`  ${nameFor(did)}: ${h.cards.join(" ")} — ${h.outcome}${payout}`);
      }
      if (p.insurance > 0) {
        addLog(
          `  ${nameFor(did)}: insurance ${p.insurance_payout > 0 ? `pays ${p.insurance_payout}` : "lost"}`,
        );
      }
    }
  }

  async function advanceRound() {
    _advanceTimer = null;
    if (!_session || _session.gameOver) return;
    addLog("Next round…");
    try {
      await _session.nextRound();
    } catch (e) {
      console.warn("nextRound failed:", e?.message || e);
    }
  }

  function shortDid(did) {
    return did?.slice(0, 12) + "…" + did?.slice(-6);
  }

  function nameFor(did) {
    return handleByDid[did] || shortDid(did);
  }

  // ─── User actions ─────────────────────────────────────────────────

  async function submitWager() {
    if (!_session || myNeed?.kind !== "wager") return;
    try {
      await _session.act(`wager:${wagerAmount}`);
    } catch (e) {
      error = "Wager failed: " + (e?.message || e);
    }
  }

  async function handleAction(action) {
    if (!_session) return;
    const verb =
      action.type === "insurance-yes"
        ? "insurance:yes"
        : action.type === "insurance-no"
          ? "insurance:no"
          : action.type;
    try {
      await _session.act(verb);
    } catch (e) {
      error = "Action failed: " + (e?.message || e);
    }
  }

  async function copyTableUri() {
    try {
      await navigator.clipboard.writeText(`${window.location.origin}/${tableUri}`);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {}
  }

  const tableHostDid = $derived(tableUri ? tableUri.split("/")[2] : "");
  const tableTid = $derived(tableUri ? tableUri.split("/").pop() : "…");

  // ─── Derived view models ──────────────────────────────────────────

  const playersByDid = $derived.by(() => {
    const m = {};
    const players = gs?.players || [];
    for (const p of players) {
      const did = playerDids[p.seat];
      if (!did) continue;
      m[did] = {
        name: nameFor(did),
        chips: p.chips,
        wager: p.wager,
        insurance: p.insurance,
        surrendered: p.surrendered,
        eliminated: p.eliminated,
        hands: (p.hands || []).map((h) => ({
          ...h,
          cards: (h.cards || []).map(parseCard).filter(Boolean),
        })),
      };
    }
    return m;
  });

  const bankerDid = $derived(gs ? playerDids[gs.banker] : null);
  const bankerCards = $derived((gs?.bankerCards || []).map(parseCard).filter(Boolean));
  const actionOnDid = $derived(gs?.actionOn != null ? playerDids[gs.actionOn] : null);

  const uiPhase = $derived.by(() => {
    if (phase === "Init") return "loading";
    if (phase === "CommitSeeds" || phase.startsWith("Shuffle") || phase.startsWith("Lock")) {
      return "shuffling";
    }
    if (phase.startsWith("Wagering")) return "wagering";
    if (phase.startsWith("Dealing")) return "dealing";
    if (phase.startsWith("Insurance")) return "insurance";
    if (phase.startsWith("PlayerTurn")) return "turn";
    if (phase === "Complete") return "results";
    return phase.toLowerCase();
  });

  const decisionActions = $derived.by(() => {
    if (myNeed?.kind === "decision") {
      return myNeed.options.map((o) => ({ type: o, label: o.toUpperCase() }));
    }
    if (myNeed?.kind === "insurance") {
      return [
        { type: "insurance-yes", label: "INSURANCE: YES" },
        { type: "insurance-no", label: "NO INSURANCE" },
      ];
    }
    return [];
  });

  const wagerQuickAmounts = $derived.by(() => {
    if (myNeed?.kind !== "wager") return [];
    const { min, max } = myNeed;
    const out = [];
    const seen = new Set();
    for (const [label, amount] of [
      ["MIN", min],
      ["×2", min * 2],
      ["×5", min * 5],
      ["MAX", max],
    ]) {
      if (amount >= min && amount <= max && !seen.has(amount)) {
        seen.add(amount);
        out.push({ label, amount });
      }
    }
    return out;
  });

  const waitingText = $derived(
    actionOnDid && actionOnDid !== session?.did
      ? `Waiting for ${nameFor(actionOnDid)}…`
      : uiPhase === "shuffling"
        ? "Shuffling the deck…"
        : uiPhase === "dealing"
          ? "Dealing…"
          : "",
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
      table: <code>{nameFor(tableHostDid)}/{tableTid}</code>
      <span class="copy-hint">{copied ? "✓ copied" : "copy"}</span>
    </button>
    <span class="phase-label" data-testid="phase">{uiPhase}</span>
    <button class="btn leave" onclick={onLeaveRoom}>Leave</button>
  </header>

  {#if error}
    <div class="error-banner">{error}</div>
  {/if}

  {#if gameOver}
    <div class="gameover-banner" data-testid="game-over">
      🏆 Game over — {winnerDid ? `${nameFor(winnerDid)} wins it all!` : "winner takes all"}
    </div>
  {/if}

  <div class="main-area">
    <aside class="log-panel" class:open={logOpen}>
      <GameLog events={logEvents} />
    </aside>

    <div class="fit-box" bind:clientWidth={fitBoxW} bind:clientHeight={fitBoxH}>
      <div class="fit-content" bind:clientHeight={fitContentH} style="transform: scale({fitScale})">
        {#if !tableRecord}
          <p class="loading">Loading table…</p>
        {:else}
          <div class="table-wrapper">
            <BlackjackTable
              players={playersByDid}
              playerOrder={playerDids}
              handleMap={handleByDid}
              {bankerDid}
              {bankerCards}
              bankerTotal={gs?.bankerTotal ?? 0}
              minBet={gs?.minBet ?? tableRecord.minBet}
              roundIndex={gs?.handIndex ?? 0}
              currentPlayer={actionOnDid}
              activeHand={gs?.actionHand ?? null}
              ourPlayerId={session?.did}
            />
          </div>

          <div class="bottom-panel">
            {#if isSpectator}
              <div class="spectating" data-testid="spectating">
                👁 Spectating{actionOnDid ? ` — ${nameFor(actionOnDid)} to act` : ""}
              </div>
            {:else if roundOver && !gameOver}
              <div class="round-over" data-testid="round-result">
                Round {(gs?.handIndex ?? 0) + 1} finished — next round shortly…
              </div>
            {:else if myNeed?.kind === "wager"}
              <div class="wager-panel" data-testid="wager-panel">
                <div class="wager-quick">
                  {#each wagerQuickAmounts as q}
                    <button
                      class="quick-btn"
                      class:active={wagerAmount === q.amount}
                      onclick={() => (wagerAmount = q.amount)}
                    >
                      {q.label}<br /><span class="quick-val">{q.amount}</span>
                    </button>
                  {/each}
                </div>
                <div class="wager-row">
                  <input
                    type="range"
                    min={myNeed.min}
                    max={myNeed.max}
                    bind:value={wagerAmount}
                    data-testid="wager-input"
                  />
                  <span class="wager-val">{wagerAmount}</span>
                  <button class="btn primary" onclick={submitWager} data-testid="wager-submit">
                    WAGER {wagerAmount}
                  </button>
                </div>
              </div>
            {:else}
              <ActionBar
                actions={decisionActions}
                onAction={handleAction}
                isOurTurn={decisionActions.length > 0}
                placeholder={waitingText}
              />
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </div>

  <button class="log-toggle" onclick={() => (logOpen = !logOpen)} data-testid="log-toggle">
    {logOpen ? "▼ hide log" : "▲ log"}
  </button>
</div>

<style>
  .game-room {
    height: 100dvh;
    overflow: hidden;
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
  .btn.primary {
    background: #c0392b;
    color: #ffffff;
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
    min-height: 0;
    display: flex;
    overflow: hidden;
  }
  .fit-box {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .fit-content {
    flex: none;
    width: 900px; /* DESIGN_W */
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 0.75rem;
  }
  .table-wrapper {
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .bottom-panel {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    max-width: 750px;
    margin: 0 auto;
    width: 100%;
  }
  .spectating,
  .round-over {
    text-align: center;
    font-size: 0.45rem;
    color: #1a1a1a;
    opacity: 0.75;
    padding: 0.75rem;
  }

  .wager-panel {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.5rem 0.75rem 0.75rem;
  }
  .wager-quick {
    display: flex;
    gap: 0.4rem;
    justify-content: center;
  }
  .quick-btn {
    padding: 0.35rem 0.5rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    font-family: inherit;
    font-size: 0.42rem;
    cursor: pointer;
    background: #ffffff;
    color: #1a1a1a;
    box-shadow: 2px 2px 0 #1a1a1a;
    transition: all 0.1s;
    line-height: 1.3;
  }
  .quick-btn:hover {
    transform: translate(1px, 1px);
    box-shadow: 1px 1px 0 #1a1a1a;
  }
  .quick-btn.active {
    background: #c0392b;
    color: #ffffff;
  }
  .quick-val {
    font-size: 0.4rem;
    opacity: 0.7;
  }
  .wager-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .wager-row input[type="range"] {
    flex: 1;
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    background: #1a1a1a;
    border: none;
    outline: none;
  }
  .wager-row input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 14px;
    height: 14px;
    background: #c0392b;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    cursor: pointer;
  }
  .wager-val {
    font-size: 0.4rem;
    color: #1a1a1a;
    min-width: 2.2rem;
    text-align: right;
  }

  /* ── Game log placement (same responsive split as GameRoom) ── */
  .log-panel {
    background: #ffffff;
  }
  @media (orientation: landscape) {
    .log-panel {
      flex: none;
      width: 260px;
      min-height: 0;
      display: flex;
      flex-direction: column;
      padding: 0.5rem;
      border-right: 3px solid #1a1a1a;
    }
    .log-panel :global(.game-log) {
      flex: 1;
      min-height: 0;
      max-height: none;
    }
    .log-toggle {
      display: none;
    }
  }
  @media (orientation: portrait) {
    .log-panel {
      position: fixed;
      left: 0;
      right: 0;
      bottom: 0;
      z-index: 20;
      padding: 0.5rem;
      border-top: 3px solid #1a1a1a;
      transform: translateY(100%);
      transition: transform 0.25s ease;
    }
    .log-panel.open {
      transform: translateY(0);
    }
    .log-panel :global(.game-log) {
      max-height: 40dvh;
    }
    .log-toggle {
      position: fixed;
      right: 0.6rem;
      bottom: 0.6rem;
      z-index: 21;
      font-family: inherit;
      font-size: 0.4rem;
      letter-spacing: 1px;
      padding: 0.4rem 0.7rem;
      background: #1a1a1a;
      color: #ffffff;
      border: 2px solid #1a1a1a;
      border-radius: 0;
      cursor: pointer;
      box-shadow: 3px 3px 0 rgba(26, 26, 26, 0.4);
    }
    .log-toggle:hover {
      background: #c0392b;
    }
  }
</style>
