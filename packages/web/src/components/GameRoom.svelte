<script>
  import { onMount, onDestroy } from "svelte";
  import * as dagCbor from "@ipld/dag-cbor";
  import PokerTable from "./PokerTable.svelte";
  import ActionBar from "./ActionBar.svelte";
  import GameLog from "./GameLog.svelte";
  import { initWasm, parseCard, createAgent } from "../lib/cardcore-wasm.js";
  import { PlayerSession, generateSeed } from "../lib/game-session.js";
  import { Publisher, fetchTableRecord } from "../lib/transport.js";
  import { FirehoseSubscriber } from "../lib/firehose.js";
  import { LEXICONS } from "../lib/atproto-publisher.js";
  import { GAME_PHASES } from "../lib/poker-engine.js";
  import { resolveHandles } from "../lib/atproto.js";
  import { getSetting, setSetting } from "../lib/settings.js";
  import { playTurnCue } from "../lib/audio.js";
  import AtbloonsHandoff from "./AtbloonsHandoff.svelte";
  import { lookupContractForTable } from "../lib/atbloons/contract-lookup.js";
  import { ChainedGame } from "../lib/atbloons/chained-game.js";
  import { isAtbloonsEnabled } from "../lib/atbloons/config.js";

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
  // Pending protocol steps from the agent's state machine: [{kind, seats,
  // deckPosition?}]. Shown at all times so a stalled game says exactly whose
  // action it's waiting for — comparing this line across windows is how we
  // debug "everyone committed but nobody shuffled".
  let waitingOn = $state([]);
  let copied = $state(false);
  let gameOver = $state(false);
  let winnerDid = $state(null);
  let isSpectator = $state(false);
  // atbloons paid-hand references: the finalized table strong ref and the most
  // recent action strong ref (a settle candidate). Both stay null for unpaid
  // games, where the atbloons panel renders nothing.
  let atbloonsTableRef = $state(null);
  let lastActionRef = $state(null);
  // Paid (settlement-valid) mode. When the managed wallet is configured this
  // seat plays the hand as ONE bounded, globally ordered action chain
  // (`ChainedGame`) instead of the unpaid per-author `PlayerSession`, so the
  // atbloons node can re-verify and settle it. Decided once at mount; unpaid
  // play is byte-for-byte unchanged when no wallet is configured.
  let paidMode = $state(false);
  // The paid hand reached its Complete phase (the mandatory seed-reveal tail is
  // published). The global chain tip is then the terminal action to settle.
  let paidComplete = $state(false);
  let chainTipRef = $state(null);
  // A non-host seat discovers the contract from the host repo (seat 0) so it
  // funds without pasting a strong reference. Returns null for unpaid games or
  // when the host has not proposed yet; the panel then keeps manual entry.
  async function atbloonsContractLookup() {
    const hostDid = playerDids?.[0];
    if (!hostDid || !atbloonsTableRef || !session?.client) return null;
    return lookupContractForTable({
      client: session.client,
      hostDid,
      tableRef: atbloonsTableRef,
    });
  }
  // We're in the roster, but this device doesn't hold the game's key
  // material (the seed lives in localStorage on whatever device played it).
  // Without the seed we can't reproduce our published actions — so we watch
  // as a spectator instead of fabricating fresh keys and diverging.
  let keyless = $state(false);
  // Portrait-mode game log sheet (slides up from the bottom).
  let logOpen = $state(false);
  // Device-local settings (localStorage): sound cue when it's your turn.
  let settingsOpen = $state(false);
  let soundOnTurn = $state(getSetting("turnSound", true));
  let _wasOurTurn = false;

  function setSoundOnTurn(on) {
    soundOnTurn = on;
    setSetting("turnSound", on);
  }

  // One-shot pre-action armed while it's NOT our turn, like online poker's
  // "Call 100" / "Call Any" checkboxes. Fires automatically when action
  // reaches us; a "call N" arm is dropped if the amount changed (a raise
  // behind us must never be called blind).
  let preAction = $state(null); // null | {type:"call", amount} | {type:"callAny"}
  let ourToCall = $state(0);
  let ourFolded = $state(false);

  // ─── Scale-to-fit ───────────────────────────────────────────────
  // The play area renders at a fixed design width and is uniformly scaled
  // (transform) so the whole game always fits the viewport — no scrolling.
  // Measured sizes are layout sizes, which transforms don't affect, so this
  // doesn't feed back into itself.
  const DESIGN_W = 900;
  let fitBoxW = $state(0);
  let fitBoxH = $state(0);
  let fitContentH = $state(0);
  const fitScale = $derived.by(() => {
    if (!fitBoxW || !fitBoxH || !fitContentH) return 1;
    return Math.min(fitBoxW / DESIGN_W, fitBoxH / fitContentH, 2);
  });

  // Short pause between hands — the deal starts almost immediately, and the
  // result banner (below) stays up on its own timer so there's still time to
  // read what happened while the next hand is dealt underneath.
  const NEXT_HAND_DELAY = 1000;
  // How long the hand-result banner stays up. Dismissible early.
  const HAND_BANNER_MS = 7000;

  // Previous hand's result, shown as an overlay banner with a countdown while
  // the next hand deals underneath: { key, title, lines, expiresAt }.
  let handBanner = $state(null);
  let _bannerNow = $state(0);
  let _bannerTimer = null;
  const bannerSecondsLeft = $derived(
    handBanner ? Math.max(0, Math.ceil((handBanner.expiresAt - _bannerNow) / 1000)) : 0,
  );

  let _publisher = null;
  let _session = null;
  // The paid-hand engine (null for unpaid play). Exactly one of `_session` /
  // `_chained` is ever non-null; `eng()` returns whichever is active so the
  // shared UI reads the same state surface from both.
  let _chained = null;
  let _firehose = null;
  let _tableTid = null;
  let _tableCid = null;
  let _loggedHandIndex = -1;
  let _advanceTimer = null;

  let _logId = 0;
  // `protocol: true` marks noninteractive protocol steps (commits, shuffles,
  // reveals). The log folds runs of them into collapsible groups — they'd
  // otherwise drown out the human-readable story of the hand, but they must
  // stay inspectable.
  function addLog(msg, opts = {}) {
    logEvents = [...logEvents, { id: _logId++, text: msg, protocol: !!opts.protocol }];
    if (logEvents.length > 80) logEvents = logEvents.slice(-80);
  }

  const PROTOCOL_KINDS = new Set([
    "commitSeed",
    "shuffleDeck",
    "lockDeck",
    "revealLockKey",
    "verifySeed",
  ]);

  function isProtocolAction(cbor) {
    try {
      const kind = (dagCbor.decode(cbor).$type || "").split("#").pop();
      return PROTOCOL_KINDS.has(kind);
    } catch {
      return false;
    }
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
      const { record, cid } = await fetchTableRecord(tableUri, session.pdsUri);
      tableRecord = record;
      _tableCid = cid;
      // The finalized table strong ref drives an atbloons PROPOSE/FUND intent.
      atbloonsTableRef = { uri: tableUri, cid };
      _tableTid = tidFromUri(tableUri);
      playerDids = record.players;
      ourPlayerIndex = playerDids.indexOf(session.did);
      // Not in the roster → spectate. The agent replays the whole game from
      // the players' public PDS records; it just never gets a seat or a say.
      isSpectator = ourPlayerIndex < 0;
      addLog(
        `Table loaded — ${playerDids.length} players, ${record.startingChips} chips, ${record.smallBlind} SB`,
      );
      if (isSpectator) addLog("Spectating — replaying the game from PDS records…");

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

      _publisher = new Publisher({ client: session.client, did: session.did });

      // Paid mode is a table-wide property: it changes how EVERY action is
      // numbered and linked, so it must be decided before the agent starts.
      // The managed wallet being configured is the signal that this deployment
      // plays settlement-valid (chained) hands.
      paidMode = isAtbloonsEnabled();

      // getRecord strips $type; add it back for the lexicon.
      const tableForCbor = { $type: LEXICONS.TABLE, ...record };
      const tableCbor = dagCbor.encode(tableForCbor);

      // Seed selection — the load-bearing step for multi-device identity.
      // Snapshot our own already-published actions BEFORE the agent starts
      // emitting: a resumed session re-derives its deterministic history, and
      // the snapshot lets the publisher verify those re-derivations by CID
      // locally (and skip the wire) instead of re-publishing every slot.
      //
      // If we've already published actions for this table, this device must
      // hold the seed that PRODUCED them. A missing or mismatched seed (the
      // game was played on another computer) means we cannot participate —
      // fabricating a fresh seed would emit actions that diverge from our own
      // published history. Verify the stored seed reproduces our published
      // seq-0 commitSeed; otherwise become a spectator.
      let seed;
      if (isSpectator) {
        // True spectators get a throwaway seed — their agent never emits.
        seed = generateSeed();
      } else {
        const stored = loadStoredSeed(tableUri);
        let publishedCount = 0;
        try {
          publishedCount = await _publisher.loadPublishedActions(_tableTid);
        } catch (e) {
          // Non-fatal: occupied slots fall back to create-then-probe.
          console.warn("loadPublishedActions failed:", e?.message || e);
        }
        if (publishedCount > 0) {
          // Our own commitSeed sits at global seq = our seat index in a paid
          // (chained) hand, but at our own seq 0 in an unpaid per-author hand.
          const commitSeq = paidMode ? ourPlayerIndex : 0;
          const seq0 = _publisher.publishedRecord(_tableTid, commitSeq);
          if (stored && seq0 && seedReproducesCommit(stored, tableCbor, seq0.value)) {
            seed = stored;
            addLog(`Resuming — ${publishedCount} of our actions already published`);
          } else {
            isSpectator = true;
            keyless = true;
            seed = generateSeed();
            addLog(
              "⚠ This game's key material isn't on this device — " +
                "watching as a spectator. To play, use the device you started on.",
            );
          }
        } else {
          seed = stored ?? createAndStoreSeed(tableUri);
        }
      }

      // A keyless viewer's DID is in the roster, but its agent must NOT take
      // the seat — a seated agent auto-emits (unpaid) or auto-produces (paid)
      // crypto derived from the (throwaway) seed and its self-records would
      // shadow the real ones. A non-roster DID makes the agent a pure replay
      // observer in both modes.
      const engineDid = isSpectator ? "did:cardcore:viewer" : session.did;
      if (paidMode) {
        // Paid: one bounded, globally ordered action chain the atbloons node
        // can re-verify and settle. The driver auto-produces every
        // deterministic step (commit, shuffle, lock, reveals, and the mandatory
        // final seed-reveal) and pauses only for a human bet; a viewer never
        // produces and just replays the chain in global order.
        _chained = new ChainedGame({
          agent: createAgent(engineDid, seed),
          did: engineDid,
          tableRef: { uri: tableUri, cid: _tableCid },
          tableTid: _tableTid,
          publisher: _publisher,
          onUpdate: refreshUi,
          onPublish: (ref, _seq, cbor) => {
            addLog(`You: ${actionLabel(cbor)}`, { protocol: isProtocolAction(cbor) });
            if (ref?.uri && ref?.cid) lastActionRef = ref;
          },
        });
        addLog(isSpectator ? "Watching paid table…" : "Joining paid table…");
        await _chained.receiveTable(tableCbor);
      } else {
        _session = new PlayerSession({
          did: engineDid,
          seed,
          publishAction: isSpectator
            ? async () => {}
            : async ({ seq, cbor }) => {
                addLog(`You: ${actionLabel(cbor)}`, { protocol: isProtocolAction(cbor) });
                const published = await _publisher.publishAction({
                  tableRef: { uri: tableUri, cid: _tableCid },
                  seq,
                  tableTid: _tableTid,
                  actionCbor: cbor,
                });
                // Remember the latest action ref as a settle candidate for the
                // atbloons paid-hand panel. Unused for unpaid games.
                if (published?.uri && published?.cid) lastActionRef = published;
              },
          onUpdate: refreshUi,
        });

        // Feed the table to our local agent first — that moves it out of Init
        // and into the CommitSeeds phase, so the firehose backfill (which may
        // include peer CommitSeeds already on disk) won't be rejected as
        // out-of-phase.
        addLog(isSpectator ? "Watching table…" : "Joining table…");
        await _session.receiveTable(tableCbor);
      }

      // Subscribe to EVERY player's repo — including our own. Replaying our
      // own records is what makes a page reload resumable: re-derivable
      // actions arrive as duplicates and are dropped, while our past bets
      // (human choices, not re-derivable from the seed) re-apply.
      _firehose = new FirehoseSubscriber({
        peerDids: playerDids,
        tableUri,
        ownPdsUri: session.pdsUri,
        onAction: async (did, seq, cbor, record) => {
          // A keyless viewer's own past records are REPLAY input, not echo:
          // the viewer agent never emitted them, so they must apply like any
          // peer's. fromSelf semantics only exist for a seated session.
          const fromSelf = !isSpectator && did === session.did;
          // Own records are already logged at publish time.
          if (!fromSelf) {
            addLog(`${nameFor(did)}: ${actionLabel(cbor)}`, { protocol: isProtocolAction(cbor) });
          }
          try {
            if (paidMode) {
              // Route the DECODED action record into the global chain: the
              // driver applies it in strict global order and content-addresses
              // it for the next `prev` link. Own echoes of already-passed slots
              // are dropped; our unplayed slots on resume re-apply in order.
              await _chained.deliverFirehoseAction(did, seq, cbor, record);
            } else {
              await _session.receiveAction(cbor, { did, fromSelf, seq });
            }
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
    if (_bannerTimer) clearInterval(_bannerTimer);
    _firehose?.stop();
    _session?.destroy();
    _chained?.destroy();
  });

  // The active game engine: the paid chain when in paid mode, else the unpaid
  // per-author session. Both expose the same read-only state surface
  // (gameState, rawHoleCards, phase, waitingOn, needsBet, betOptions,
  // isComplete, lastHandResult, gameOver, pendingCount) so the UI reads one.
  function eng() {
    return _chained || _session;
  }

  // ─── Helpers ──────────────────────────────────────────────────────

  function loadStoredSeed(uri) {
    const stored = localStorage.getItem(`cardcore_seed:${uri}`);
    if (stored) {
      const arr = stored.split(",").map(Number);
      if (arr.length === 32) return new Uint8Array(arr);
    }
    return null;
  }

  function createAndStoreSeed(uri) {
    const seed = generateSeed();
    localStorage.setItem(`cardcore_seed:${uri}`, Array.from(seed).join(","));
    return seed;
  }

  /**
   * Whether `seed` is the key material this game was actually played with:
   * a throwaway agent is given the seed and the table, and the commitSeed it
   * derives must match the one we PUBLISHED at seq 0. Anything else means the
   * seed can't reproduce our own history and must not be allowed to play.
   */
  function seedReproducesCommit(seed, tableCbor, publishedValue) {
    let probe = null;
    try {
      probe = createAgent(session.did, seed);
      const out = probe.receive_table(tableCbor);
      if (out.kind !== "actions" || out.action_count < 1) return false;
      const derived = dagCbor.decode(new Uint8Array(out.action(0)));
      const pub = publishedValue?.action;
      if (!pub || !String(pub.$type || "").endsWith("commitSeed")) return false;
      if (!derived?.commitment) return false;
      let bin = "";
      for (const byte of derived.commitment) bin += String.fromCharCode(byte);
      // The PDS serves $bytes unpadded; btoa pads — normalize both.
      const derB64 = btoa(bin).replace(/=+$/, "");
      const pubB64 = String(pub.commitment?.$bytes || "").replace(/=+$/, "");
      return derB64 === pubB64;
    } catch {
      return false;
    } finally {
      try {
        probe?.free();
      } catch {}
    }
  }

  function refreshUi() {
    const engine = eng();
    if (!engine) return;
    const gs = engine.gameState;
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

      ourToCall = Math.max(0, (gs.currentBet ?? 0) - (bets[session.did] ?? 0));
      ourFolded = !!folded[session.did];
      // A raise behind us invalidates a "call N" arm; folding clears any arm.
      if (preAction?.type === "call" && ourToCall !== preAction.amount) preAction = null;
      if (ourFolded) preAction = null;
    }
    holeCards = (engine.rawHoleCards || []).map(parseCard).filter(Boolean);
    communityCards = (engine.rawCommunityCards || []).map(parseCard).filter(Boolean);
    phase = engine.phase;
    waitingOn = engine.waitingOn;

    const commLen = communityCards.length;
    let uiPhase = "preflop";
    if (phase === "Showdown" || phase === "Complete") uiPhase = "showdown";
    else if (commLen >= 5) uiPhase = "river";
    else if (commLen >= 4) uiPhase = "turn";
    else if (commLen >= 3) uiPhase = "flop";

    if (engine.isComplete) {
      handleHandComplete();
    }

    // Audio cue on the rising edge of "it's your turn" — never on repeats.
    if (engine.needsBet && !_wasOurTurn && soundOnTurn && !isSpectator) {
      playTurnCue();
    }
    _wasOurTurn = engine.needsBet;

    // Armed pre-action: consume it now that action reached us. setTimeout
    // breaks out of the onUpdate call stack — bet() must not re-enter the
    // agent while it's mid-processing.
    if (engine.needsBet && preAction && !isSpectator) {
      const pre = preAction;
      preAction = null;
      const opts = mapBetOptions(engine.betOptions);
      const canCall = opts.some((a) => a.type === "call");
      const canCheck = opts.some((a) => a.type === "check");
      let fire = null;
      if (pre.type === "callAny") fire = canCall ? "call" : canCheck ? "check" : null;
      else if (pre.type === "call" && canCall && ourToCall === pre.amount) fire = "call";
      if (fire) setTimeout(() => handleAction({ type: fire }), 0);
    }

    if (engine.needsBet) {
      isOurTurn = true;
      availableActions = mapBetOptions(engine.betOptions);
      const ourChips = chipsByDid[session.did] ?? 0;
      // The minimum raise TOTAL comes from the engine's suggested Raise
      // option (current bet + big blind). A fixed big-blind minimum here is
      // how an under-raise below the standing bet once reached the engine.
      const raiseOpt = availableActions.find((a) => a.type === "raise");
      raiseContext = raiseOpt
        ? {
            min: raiseOpt.amount,
            max: ourChips,
            pot,
            quickAmounts: buildQuickAmounts(pot, raiseOpt.amount, ourChips),
          }
        : null;
    } else {
      isOurTurn = false;
      availableActions = [];
      raiseContext = null;
    }
  }

  // Hand finished: log the result once, then either declare the game over or
  // schedule the next hand to start automatically.
  function handleHandComplete() {
    const engine = eng();
    // Replayed hands are history, not suspense: no banner, no readable pause.
    // (A live boundary has at most one pending CommitSeed per peer; a backlog
    // bigger than the roster means we're replaying.)
    const catchingUp = isSpectator
      ? engine.pendingCount > 0
      : engine.pendingCount > playerDids.length;

    preAction = null; // pre-actions never carry across a hand boundary

    const result = engine.lastHandResult;
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
      if (!catchingUp && !engine.gameOver) showHandBanner(result);
    }

    // A paid hand is ONE settlement-valid chain: never auto-advance. When the
    // chain reaches Complete (its mandatory final seed-reveal is published),
    // the global chain tip is the terminal action the settle intent uses — the
    // panel then offers one-click settlement with no hand-typed reference.
    if (paidMode) {
      if (!paidComplete) {
        paidComplete = true;
        chainTipRef = _chained.tipRef;
        addLog("Paid hand complete — settle to move the chip stacks back to atbloons.");
      }
      return;
    }

    if (engine.gameOver) {
      if (!gameOver) {
        gameOver = true;
        winnerDid = playerDids.find((d) => (chipsByDid[d] ?? 0) > 0) ?? null;
        addLog(`🏆 Game over — ${nameFor(winnerDid)} wins!`);
      }
      return;
    }

    // Auto-advance to the next hand quickly — the result banner stays up on
    // its own longer timer, so readability doesn't depend on delaying the deal.
    if (!_advanceTimer) {
      _advanceTimer = setTimeout(advanceHand, catchingUp ? 250 : NEXT_HAND_DELAY);
    }
  }

  // Overlay the previous hand's result for HAND_BANNER_MS while the next hand
  // is dealt underneath. Winners headline it; shown hands are the fine print.
  function showHandBanner(result) {
    const winners = [];
    const lines = [];
    for (const pot of result.pots || []) {
      const names = pot.winners.map((w) => nameFor(playerDids[w])).join(", ");
      if (!names) continue;
      const how = result.by_fold ? "all others folded" : pot.hand_desc || "";
      winners.push(`${names} wins ${pot.amount}${how ? ` — ${how}` : ""}`);
    }
    if (!result.by_fold) {
      for (const s of result.shown || []) {
        lines.push(`${nameFor(playerDids[s.seat])}: ${s.cards.join(" ")} — ${s.hand_desc}`);
      }
    }
    handBanner = {
      key: result.hand_index,
      title: `Hand ${result.hand_index + 1}`,
      winners,
      lines,
      expiresAt: Date.now() + HAND_BANNER_MS,
    };
    _bannerNow = Date.now();
    if (_bannerTimer) clearInterval(_bannerTimer);
    _bannerTimer = setInterval(() => {
      _bannerNow = Date.now();
      if (handBanner && _bannerNow >= handBanner.expiresAt) dismissHandBanner();
    }, 250);
  }

  function dismissHandBanner() {
    handBanner = null;
    if (_bannerTimer) {
      clearInterval(_bannerTimer);
      _bannerTimer = null;
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

  // "waiting on shuffleDeck from example.com" — always-on status built from
  // the agent's own valid_actions, covering the noninteractive steps as well
  // as bets. "you" in this line means OUR agent owes the step and hasn't
  // emitted it: that window is the stuck one.
  const waitingMsg = $derived.by(() => {
    // verifySeed is deliberately never auto-emitted in a multi-hand game (it
    // would reveal the hand's deal), so a Complete phase only means the next
    // hand is coming — don't report it as waiting on anyone.
    const entries = waitingOn.filter((e) => e.kind !== "verifySeed");
    if (!entries.length) {
      if (!gameOver && phase === "Complete") {
        // A paid hand is a single hand — there is no next hand to deal.
        return paidMode ? "hand complete — settle to finish" : "hand complete — next hand soon…";
      }
      return "";
    }
    return entries
      .map((e) => {
        const step = e.kind === "revealLockKey" ? `revealLockKey #${e.deckPosition}` : e.kind;
        const who = (e.seats || [])
          .map((s) => (playerDids[s] === session?.did ? "you" : nameFor(playerDids[s])))
          .join(", ");
        return `waiting on ${step} from ${who}`;
      })
      .join("; ");
  });

  // ─── User actions ─────────────────────────────────────────────────

  async function handleAction(action) {
    const engine = eng();
    if (!engine) return;
    let bet;
    if (action.type === "raise") bet = `raise:${action.amount || 2}`;
    else bet = action.type;
    // No addLog here — the bet is logged like every other action when its
    // record is published.
    try {
      if (paidMode) await _chained.submitBet(bet);
      else await _session.bet(bet);
    } catch (e) {
      error = "Bet failed: " + (e?.message || e);
    }
  }

  async function copyTableUri() {
    try {
      // The full shareable URL — paste it straight into a browser.
      await navigator.clipboard.writeText(`${window.location.origin}/${tableUri}`);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {}
  }

  // A tid is meaningless without its repo — label the table as host/tid.
  const tableHostDid = $derived(tableUri ? tableUri.split("/")[2] : "");
  const tableTid = $derived(tableUri ? tableUri.split("/").pop() : "…");

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
      table: <code>{nameFor(tableHostDid)}/{tableTid}</code>
      <span class="copy-hint">{copied ? "✓ copied" : "copy"}</span>
    </button>
    <span class="phase-label" data-testid="phase">{gamePhase}</span>
    <div class="settings-wrap">
      <button
        class="btn"
        onclick={() => (settingsOpen = !settingsOpen)}
        data-testid="settings-toggle"
        title="Settings"
      >
        SET
      </button>
      {#if settingsOpen}
        <div class="settings-menu" data-testid="settings-menu">
          <label class="settings-item">
            <input
              type="checkbox"
              data-testid="setting-turn-sound"
              checked={soundOnTurn}
              onchange={(e) => setSoundOnTurn(e.currentTarget.checked)}
            />
            sound when it's your turn
          </label>
          <div class="settings-note">settings are saved on this device only</div>
        </div>
      {/if}
    </div>
    <button class="btn leave" onclick={leave}>Leave</button>
  </header>

  {#if error}
    <div class="error-banner">{error}</div>
  {/if}

  {#if gameOver}
    <div class="gameover-banner" data-testid="game-over">
      🏆 Game over — {winnerDid ? `${nameFor(winnerDid)} wins it all!` : "winner takes all"}
    </div>
  {/if}

  {#if !isSpectator && ourPlayerIndex >= 0 && atbloonsTableRef}
    <AtbloonsHandoff
      tableRef={atbloonsTableRef}
      seat={ourPlayerIndex}
      terminalActionRef={paidMode
        ? paidComplete
          ? chainTipRef
          : null
        : gameOver
          ? lastActionRef
          : null}
      contractLookup={atbloonsContractLookup}
    />
  {/if}

  {#if handBanner}
    <!-- Previous hand's result, floating over the fresh deal. The next hand
         is already being dealt underneath; this is purely for readability. -->
    <div class="hand-banner" data-testid="hand-banner">
      <div class="hand-banner-head">
        <span class="hand-banner-title">{handBanner.title}</span>
        <span class="hand-banner-count" data-testid="hand-banner-count">{bannerSecondsLeft}s</span>
        <button
          class="hand-banner-dismiss"
          onclick={dismissHandBanner}
          data-testid="hand-banner-dismiss"
          title="Dismiss"
        >
          ✕
        </button>
      </div>
      {#each handBanner.winners as w}
        <div class="hand-banner-winner">🏆 {w}</div>
      {/each}
      {#each handBanner.lines as line}
        <div class="hand-banner-line">{line}</div>
      {/each}
    </div>
  {/if}

  <div class="main-area">
    <!-- Landscape: a static panel on the left of the game. Portrait: a sheet
         that slides up from the bottom, toggled by the Log button. -->
    <aside class="log-panel" class:open={logOpen}>
      <GameLog events={logEvents} />
    </aside>

    <div class="fit-box" bind:clientWidth={fitBoxW} bind:clientHeight={fitBoxH}>
      <div class="fit-content" bind:clientHeight={fitContentH} style="transform: scale({fitScale})">
        {#if !tableRecord}
          <p class="loading">Loading table…</p>
        {:else}
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
            {#if isSpectator}
              <div class="spectating" data-testid="spectating">
                {#if keyless}
                  <span data-testid="keyless-note">
                    👁 Watching only — this game's keys aren't on this device. To play, use the
                    device you started on.
                  </span>
                {:else}
                  👁 Spectating{waitingMsg ? ` — ${waitingMsg}` : ""}
                {/if}
              </div>
            {:else}
              <ActionBar
                actions={availableActions}
                raise={raiseContext}
                onAction={handleAction}
                {isOurTurn}
                placeholder={waitingMsg}
              />
              {#if !gameOver}
                <!-- Pre-actions: act automatically the moment action reaches
                     us. "Call N" only holds for that exact price. Hidden (but
                     still occupying height — the bottom panel must not change
                     size across turn changes or the scaled game jumps) when
                     it's our turn or we're out of the hand. -->
                <div
                  class="preact-row"
                  class:invisible={isOurTurn ||
                    ourFolded ||
                    phase === "Init" ||
                    phase === "Complete"}
                >
                  {#if ourToCall > 0}
                    <label class="preact">
                      <input
                        type="checkbox"
                        data-testid="preact-call"
                        checked={preAction?.type === "call"}
                        onchange={(e) =>
                          (preAction = e.currentTarget.checked
                            ? { type: "call", amount: ourToCall }
                            : null)}
                      />
                      CALL {ourToCall}
                    </label>
                  {/if}
                  <label class="preact">
                    <input
                      type="checkbox"
                      data-testid="preact-call-any"
                      checked={preAction?.type === "callAny"}
                      onchange={(e) =>
                        (preAction = e.currentTarget.checked ? { type: "callAny" } : null)}
                    />
                    CALL ANY
                  </label>
                </div>
              {/if}
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
    position: relative; /* anchors the .hand-banner overlay */
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
  .settings-wrap {
    position: relative;
  }
  .settings-menu {
    position: absolute;
    top: calc(100% + 0.4rem);
    right: 0;
    z-index: 40;
    background: #ffffff;
    border: 2px solid #1a1a1a;
    box-shadow: 3px 3px 0 #1a1a1a;
    padding: 0.5rem 0.75rem;
    min-width: 11rem;
  }
  .settings-item {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.42rem;
    color: #1a1a1a;
    cursor: pointer;
    white-space: nowrap;
  }
  .settings-note {
    margin-top: 0.4rem;
    font-size: 0.35rem;
    color: #1a1a1a;
    opacity: 0.5;
  }
  .preact-row {
    display: flex;
    gap: 1.2rem;
    justify-content: center;
    padding: 0.1rem 0.75rem 0.25rem;
    min-height: 0.9rem;
  }
  .preact-row.invisible {
    visibility: hidden;
  }
  .preact {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.42rem;
    color: #1a1a1a;
    opacity: 0.75;
    cursor: pointer;
    letter-spacing: 1px;
    white-space: nowrap;
  }
  .preact:hover {
    opacity: 1;
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
  /* Floats over the table so the next deal proceeds underneath. */
  .hand-banner {
    position: absolute;
    top: 4.2rem;
    left: 50%;
    transform: translateX(-50%);
    z-index: 30;
    min-width: 16rem;
    max-width: 90vw;
    background: #ffffff;
    border: 3px solid #1a1a1a;
    box-shadow: 4px 4px 0 #1a1a1a;
    padding: 0.6rem 0.75rem;
  }
  .hand-banner-head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin-bottom: 0.4rem;
  }
  .hand-banner-title {
    font-size: 0.45rem;
    letter-spacing: 2px;
    color: #1a1a1a;
  }
  .hand-banner-count {
    margin-left: auto;
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.5;
  }
  .hand-banner-dismiss {
    font-family: inherit;
    font-size: 0.45rem;
    background: #ffffff;
    color: #1a1a1a;
    border: 2px solid #1a1a1a;
    cursor: pointer;
    padding: 0.1rem 0.3rem;
    box-shadow: 2px 2px 0 #1a1a1a;
  }
  .hand-banner-dismiss:hover {
    background: #c0392b;
    color: #ffffff;
    border-color: #c0392b;
  }
  .hand-banner-winner {
    font-size: 0.5rem;
    color: #1a7a3a;
    padding: 0.15rem 0;
  }
  .hand-banner-line {
    font-size: 0.42rem;
    color: #1a1a1a;
    opacity: 0.75;
    padding: 0.1rem 0;
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
  /* The game renders at DESIGN_W and is transform-scaled to fit this box,
     so the whole table is always visible regardless of screen size. The
     flexbox centers the unscaled frame; the scale pulls any overflow back
     inside the box. */
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

  /* ── Game log placement ──
     Landscape: a fixed-width panel to the LEFT of the game.
     Portrait: a bottom sheet toggled by the floating Log button. */
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
      border-color: #c0392b;
    }
  }
  .spectating {
    text-align: center;
    font-size: 0.45rem;
    color: #1a1a1a;
    padding: 0.5rem;
    border: 2px dashed #1a1a1a;
    letter-spacing: 1px;
  }
</style>
