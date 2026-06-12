/**
 * BlackjackSession — drives one player's WasmBlackjackAgent through the
 * blackjack protocol.
 *
 * Mirrors PlayerSession (game-session.js): no knowledge of transport details.
 * It receives action CBOR from somewhere (firehose), feeds the local agent,
 * and asks its caller to publish whatever the agent emits.
 *
 * Lifecycle:
 *   1. construct (creates the WasmBlackjackAgent with a fresh seed)
 *   2. receiveTable(tableCbor) once — kicks off CommitSeed emission
 *   3. receiveAction(cbor) for each peer action delivered
 *   4. act("wager:25" | "insurance:no" | "hit" | ...) on UI decisions
 *
 * The session calls `publishAction({ seq, cbor })` for every action the
 * agent emits, and `onUpdate()` whenever observable state changes.
 */
import { createBlackjackAgent, parseCard } from "./cardcore-wasm.js";

export class BlackjackSession {
  /**
   * @param {object} opts
   * @param {string} opts.did
   * @param {Uint8Array} opts.seed
   * @param {(args: {seq: number, cbor: Uint8Array}) => Promise<void>} opts.publishAction
   * @param {() => void} [opts.onUpdate]
   */
  constructor({ did, seed, publishAction, onUpdate }) {
    this.did = did;
    this.agent = createBlackjackAgent(did, seed);
    this.publishAction = publishAction;
    this.onUpdate = onUpdate || (() => {});
    this.seq = 0;
    this.publishing = Promise.resolve();
    // The interactive need the agent paused on:
    //   null | { kind: "wager", min, max } | { kind: "insurance" }
    //        | { kind: "decision", options: ["hit","stand",...] }
    this._need = null;
    // Actions that arrived before our agent was in a phase to accept them
    // (e.g. a peer's next-round CommitSeed reaching us before we've advanced
    // past the previous round). Retried after every successful state change.
    this._pending = [];
    // Publish-seq slots of our OWN actions this session has emitted or
    // replayed — see PlayerSession for the full story. Identity is
    // (self, seq); payloads alone aren't unique.
    this._selfSeqsDone = new Set();
  }

  /** This player's hand(s) as parsed card objects (two after a split). */
  get myHands() {
    try {
      return JSON.parse(this.agent.my_hands()).map((h) => h.map(parseCard).filter(Boolean));
    } catch {
      return [];
    }
  }

  /** The banker's face-up cards as parsed card objects. */
  get bankerCards() {
    try {
      return JSON.parse(this.agent.banker_cards()).map(parseCard).filter(Boolean);
    } catch {
      return [];
    }
  }

  get phase() {
    try {
      return this.agent.phase();
    } catch {
      return "Init";
    }
  }

  get isComplete() {
    return this.phase === "Complete";
  }

  get gameState() {
    try {
      return JSON.parse(this.agent.game_state());
    } catch {
      return null;
    }
  }

  /** The interactive need the agent paused on (see constructor docs). */
  get need() {
    return this._need;
  }

  // ─── Observable state ───────────────────────────────────────────────

  get needsAction() {
    return this._need != null;
  }

  /** Number of received actions still waiting for a phase that accepts them. */
  get pendingCount() {
    return this._pending.length;
  }

  /** Result of the most recently completed round, or null. */
  get lastRoundResult() {
    try {
      const json = this.agent.last_round_result();
      return json ? JSON.parse(json) : null;
    } catch {
      return null;
    }
  }

  /** Whether the whole game is over (one player holds all the chips). */
  get gameOver() {
    try {
      return this.agent.game_over();
    } catch {
      return false;
    }
  }

  async receiveTable(tableCbor) {
    const out = this.agent.receive_table(tableCbor);
    await this._processOutput(out);
    await this._drainPending();
  }

  /**
   * Feed an action record. `fromSelf`/`seq` identify records from our own
   * repo (live echo or reload replay): slots we've already emitted are
   * dropped, while ones that apply (our past wagers/decisions — human
   * choices, not re-derivable from the seed) claim their original
   * publish-seq slot so future rkeys don't collide with existing records.
   */
  async receiveAction(actionCbor, { fromSelf = false, seq = null } = {}) {
    if (fromSelf && this._selfSeqsDone.has(seq)) return;
    try {
      const out = this.agent.receive_action(actionCbor);
      if (fromSelf) {
        this._selfSeqsDone.add(seq);
        this.seq = Math.max(this.seq, seq + 1);
      }
      await this._processOutput(out);
    } catch (e) {
      // Not valid in our current phase yet — buffer and retry on the next
      // successful transition rather than dropping it (the firehose won't
      // redeliver, so dropping would deadlock the round boundary).
      this._pending.push({ cbor: actionCbor, fromSelf, seq });
      return;
    }
    await this._drainPending();
  }

  /**
   * Submit a player action: "wager:AMOUNT", "insurance:yes", "insurance:no",
   * "hit", "stand", "double", "split", or "surrender".
   */
  async act(action) {
    const out = this.agent.act(action);
    await this._processOutput(out);
    await this._drainPending();
  }

  /**
   * Advance to the next round after the current one is Complete. Emits this
   * player's CommitSeed for the new round. No-op if the game is over.
   */
  async nextRound() {
    const out = this.agent.next_round();
    await this._processOutput(out);
    await this._drainPending();
  }

  /** Retry buffered actions until none can make progress. */
  async _drainPending() {
    let progress = true;
    while (progress && this._pending.length) {
      progress = false;
      const queue = this._pending;
      this._pending = [];
      for (const item of queue) {
        if (item.fromSelf && this._selfSeqsDone.has(item.seq)) {
          // Re-derived and emitted while this copy sat in the buffer — drop.
          progress = true;
          continue;
        }
        try {
          const out = this.agent.receive_action(item.cbor);
          if (item.fromSelf) {
            this._selfSeqsDone.add(item.seq);
            this.seq = Math.max(this.seq, item.seq + 1);
          }
          await this._processOutput(out);
          progress = true;
        } catch {
          this._pending.push(item);
        }
      }
    }
  }

  async _processOutput(output) {
    // Drain emitted actions, re-checking after each batch — emitting an
    // action can advance us into our OWN turn (e.g. our reveal completes the
    // deal that ends on our decision).
    while (output.kind === "actions") {
      // Serialize publishes so putRecord calls land on the PDS in seq order.
      for (let i = 0; i < output.action_count; i++) {
        const cbor = new Uint8Array(output.action(i));
        const mySeq = this.seq++;
        // Emitted slots count as done — the copy that comes back from our
        // own repo (live echo or reload replay) must not re-apply.
        this._selfSeqsDone.add(mySeq);
        this.publishing = this.publishing.then(() => this.publishAction({ seq: mySeq, cbor }));
      }
      await this.publishing;
      this._need = null;
      output = this.agent.check_status();
    }

    if (output.kind === "need_wager") {
      let bounds = {};
      try {
        bounds = JSON.parse(output.options);
      } catch {}
      this._need = { kind: "wager", min: bounds.min ?? 1, max: bounds.max ?? 1 };
    } else if (output.kind === "need_insurance") {
      this._need = { kind: "insurance" };
    } else if (output.kind === "need_decision") {
      let options = [];
      try {
        options = JSON.parse(output.options);
      } catch {}
      this._need = { kind: "decision", options };
    } else {
      this._need = null;
    }
    this.onUpdate();
  }

  destroy() {
    if (this.agent) {
      try {
        this.agent.free();
      } catch {}
      this.agent = null;
    }
  }
}
