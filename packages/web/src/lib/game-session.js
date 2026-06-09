/**
 * PlayerSession — drives one player's WasmAgent through the poker protocol.
 *
 * The session has no knowledge of transport details. It receives action CBOR
 * from somewhere (an ActionPoller, a firehose, whatever), feeds the local
 * WasmAgent, and asks its caller to publish whatever the agent emits.
 *
 * Lifecycle:
 *   1. construct (creates the WasmAgent with a fresh seed)
 *   2. receiveTable(tableCbor) once — kicks off CommitSeed emission
 *   3. receiveAction(cbor) for each peer action delivered
 *   4. bet(action) when the UI receives a betting decision
 *
 * The session calls `publishAction({ seq, cbor })` for every action the
 * WasmAgent emits, and `onUpdate()` whenever observable state changes.
 */
import { createAgent, parseCard } from "./cardcore-wasm.js";

export class PlayerSession {
  /**
   * @param {object} opts
   * @param {string} opts.did
   * @param {Uint8Array} opts.seed
   * @param {(args: {seq: number, cbor: Uint8Array}) => Promise<void>} opts.publishAction
   * @param {() => void} [opts.onUpdate]
   */
  constructor({ did, seed, publishAction, onUpdate }) {
    this.did = did;
    this.agent = createAgent(did, seed);
    this.publishAction = publishAction;
    this.onUpdate = onUpdate || (() => {});
    this.seq = 0;
    this.publishing = Promise.resolve();
    this._needsBet = false;
    this._betOptions = [];
    // Actions that arrived before our agent was in a phase to accept them
    // (e.g. a peer's next-hand CommitSeed reaching us before we've advanced
    // past the previous hand). Retried after every successful state change.
    this._pending = [];
  }

  async receiveTable(tableCbor) {
    const out = this.agent.receive_table(tableCbor);
    await this._processOutput(out);
    await this._drainPending();
  }

  async receiveAction(actionCbor) {
    try {
      const out = this.agent.receive_action(actionCbor);
      await this._processOutput(out);
    } catch (e) {
      // Not valid in our current phase yet — buffer and retry on the next
      // successful transition rather than dropping it (the firehose won't
      // redeliver, so dropping would deadlock the hand boundary).
      this._pending.push(actionCbor);
      return;
    }
    await this._drainPending();
  }

  async bet(action) {
    const out = this.agent.bet(action);
    await this._processOutput(out);
    await this._drainPending();
  }

  /**
   * Advance to the next hand after the current one is Complete. Emits this
   * player's CommitSeed for the new hand. No-op if the game is over.
   */
  async nextHand() {
    const out = this.agent.next_hand();
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
      for (const cbor of queue) {
        try {
          const out = this.agent.receive_action(cbor);
          await this._processOutput(out);
          progress = true;
        } catch {
          this._pending.push(cbor);
        }
      }
    }
  }

  async _processOutput(output) {
    // Drain emitted actions, re-checking after each batch. Emitting an action
    // can advance us into our OWN turn — e.g. when we contribute the reveal
    // that completes a community deal and we're first to act on the new street.
    // The agent reports actions XOR a bet prompt, so without re-polling we'd
    // never surface that bet and the table would stall.
    while (output.kind === "actions") {
      // Serialize publishes so putRecord calls land on the PDS in seq order.
      for (let i = 0; i < output.action_count; i++) {
        const cbor = new Uint8Array(output.action(i));
        const mySeq = this.seq++;
        this.publishing = this.publishing.then(() => this.publishAction({ seq: mySeq, cbor }));
      }
      await this.publishing;
      this._needsBet = false;
      this._betOptions = [];
      output = this.agent.check_status();
    }

    if (output.kind === "need_bet") {
      this._needsBet = true;
      try {
        this._betOptions = JSON.parse(output.bet_options);
      } catch {
        this._betOptions = [];
      }
    } else {
      this._needsBet = false;
      this._betOptions = [];
    }
    this.onUpdate();
  }

  // ─── Observable state ───────────────────────────────────────────────

  get holeCards() {
    try {
      return JSON.parse(this.agent.hole_cards()).map(parseCard).filter(Boolean);
    } catch {
      return [];
    }
  }

  get communityCards() {
    try {
      return JSON.parse(this.agent.community_cards()).map(parseCard).filter(Boolean);
    } catch {
      return [];
    }
  }

  get rawHoleCards() {
    try {
      return JSON.parse(this.agent.hole_cards());
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

  get needsBet() {
    return this._needsBet;
  }

  get betOptions() {
    return this._betOptions;
  }

  /** Number of received actions still waiting for a phase that accepts them. */
  get pendingCount() {
    return this._pending.length;
  }

  /** Result of the most recently completed hand, or null. */
  get lastHandResult() {
    try {
      const json = this.agent.last_hand_result();
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

  destroy() {
    if (this.agent) {
      try {
        this.agent.free();
      } catch {}
      this.agent = null;
    }
  }
}

/** Generate a 32-byte random seed. */
export function generateSeed() {
  const seed = new Uint8Array(32);
  crypto.getRandomValues(seed);
  return seed;
}
