/**
 * ChainedSession — drives ONE seat through a paid (settlement-valid) poker
 * hand as a single, bounded, globally ordered action chain.
 *
 * This is the paid-hand counterpart to the unpaid `PlayerSession`
 * (`../game-session.js`). The unpaid path lets every agent auto-emit and number
 * only its OWN actions, with no cross-author `prev` link — fine for play, but
 * NOT settlement-valid: the atbloons `CARDCORE_POKER_V1` transcript evaluator
 * requires one contiguous chain where every record's `prev` points at the
 * previous action in the GLOBAL order (often in another player's repo), `seq`
 * is the global position, and the tail is every seat's `VerifySeed`.
 *
 * The engine (Rust/WASM, `set_chained(true)`) never auto-emits in this mode.
 * Instead this driver:
 *   1. asks `next_action()` who acts next in the canonical global order
 *      (`state.valid_actions()[0]` — the exact order the offline transcript
 *      generator uses and the evaluator validates);
 *   2. when it is our turn, builds the record WITHOUT applying it
 *      (`produce_next()` / `produce_bet()`), publishes it with the running
 *      global `prev`/`seq`, then feeds it back through `receive_action` so our
 *      state advances exactly like every peer's;
 *   3. when it is a peer's turn, waits for that global slot to arrive from the
 *      firehose, applies it in order, and advances the shared head.
 *
 * The driver is transport-agnostic and side-effect-free apart from the injected
 * `publish` callback: unit tests drive it with a fake agent and fake publisher,
 * and a Node integration test drives it with real WASM agents.
 *
 * Unpaid play is untouched — nothing here runs unless a paid hand is started.
 */

/** Wrap a produced Uint8Array-ish CBOR into a plain Uint8Array. */
function asBytes(value) {
  return value instanceof Uint8Array ? value : new Uint8Array(value);
}

export class ChainedSession {
  /**
   * @param {object} opts
   * @param {object} opts.agent - a chained WASM agent (set_chained already true,
   *   or set here): needs next_action(), produce_next(), produce_bet(action),
   *   receive_action(cbor, did), receive_table(cbor), phase(), set_chained(on).
   * @param {string} opts.did - this seat's DID (author attribution).
   * @param {{uri: string, cid: string}} opts.tableRef - the finalized table.
   * @param {string} opts.tableTid - the table TID (rkey prefix).
   * @param {(args: {tableRef, prevRef, seq, tableTid, actionCbor}) =>
   *   Promise<{uri: string, cid: string}>} opts.publish - publishes one action
   *   record and returns its strong ref.
   * @param {() => void} [opts.onUpdate] - called on observable state changes.
   */
  constructor({ agent, did, tableRef, tableTid, publish, onUpdate }) {
    if (!agent) throw new Error("ChainedSession requires an agent");
    if (!did) throw new Error("ChainedSession requires a did");
    if (typeof publish !== "function")
      throw new Error("ChainedSession requires a publish callback");
    this.agent = agent;
    this.did = did;
    this.tableRef = tableRef;
    this.tableTid = tableTid;
    this.publish = publish;
    this.onUpdate = onUpdate || (() => {});
    // The single global head shared by ALL authors: the prev ref and seq the
    // NEXT action in the chain must carry. Advanced for every applied action,
    // ours or a peer's.
    this.head = { prevRef: null, seq: 0 };
    // Ordered record of the produced/observed chain: { seq, seat, kind, ref }.
    this.chain = [];
    // Firehose-delivered actions keyed by global seq, awaiting their turn.
    this._pending = new Map();
    // A human betting decision waiting to be produced on our turn.
    this._pendingBet = null;
    this._needsBet = false;
    this._betOptions = [];
    this._complete = false;
    // Serialize pumps so publishes land and head advances in strict order.
    this._running = Promise.resolve();
    if (typeof agent.set_chained === "function") agent.set_chained(true);
  }

  /** Feed the table record. Starts the chain (first canonical action is a commit). */
  async receiveTable(tableCbor) {
    this.agent.receive_table(tableCbor);
    await this._pump();
  }

  /**
   * Deliver an action observed on the firehose at global `seq`, authored by
   * `did`, with its strong ref. Out-of-order arrivals are buffered until their
   * slot is reached. Our own echoes (seq already passed) are ignored.
   */
  async receiveChainAction(actionCbor, { did, seq, ref }) {
    if (seq < this.head.seq) return; // already applied — stale echo
    this._pending.set(seq, { cbor: asBytes(actionCbor), did, ref });
    await this._pump();
  }

  /** Provide a human betting decision ("fold"/"check"/"call"/"allIn"/"raise:N"). */
  async submitBet(betString) {
    this._pendingBet = betString;
    await this._pump();
  }

  async _pump() {
    this._running = this._running.then(() => this._pumpInner());
    await this._running;
  }

  async _pumpInner() {
    // Advance the chain as far as local knowledge allows: emit our own
    // canonical actions, apply buffered peer actions in slot order, and stop
    // when we need a human bet or are waiting on a peer's slot.
    for (;;) {
      const nextJson = this.agent.next_action();
      if (nextJson === "null") {
        this._complete = this.agent.phase() === "Complete";
        this._needsBet = false;
        this._betOptions = [];
        this.onUpdate();
        return;
      }
      const next = JSON.parse(nextJson);

      if (next.mine) {
        // Resume/replay: when the firehose has already delivered OUR own
        // record for this slot (a page reload replays our own repo), apply
        // it in place instead of re-producing or re-prompting. This keeps a
        // resumed paid hand from asking again for a bet we already
        // published — our past bets are human choices, not re-derivable, so
        // they must re-apply like any peer's, in global order.
        const echo = this._pending.get(this.head.seq);
        if (echo) {
          this._pending.delete(this.head.seq);
          this._needsBet = false;
          this._betOptions = [];
          this.agent.receive_action(echo.cbor, echo.did);
          this._advanceHead(echo.ref, next);
          continue;
        }
        if (next.kind === "bet") {
          if (this._pendingBet == null) {
            // Surface the decision to the UI and wait.
            this._needsBet = true;
            this._betOptions = next.options || [];
            this.onUpdate();
            return;
          }
          const bet = this._pendingBet;
          this._pendingBet = null;
          this._needsBet = false;
          this._betOptions = [];
          const cbor = asBytes(this.agent.produce_bet(bet));
          await this._publishAndApply(cbor, next);
        } else {
          const cbor = asBytes(this.agent.produce_next());
          if (cbor.length === 0) {
            // Defensive: engine says ours but produced nothing.
            this.onUpdate();
            return;
          }
          await this._publishAndApply(cbor, next);
        }
      } else {
        // A peer's slot: apply it if the firehose has delivered it.
        const peer = this._pending.get(this.head.seq);
        if (!peer) {
          this.onUpdate();
          return; // wait for the firehose
        }
        this._pending.delete(this.head.seq);
        this.agent.receive_action(peer.cbor, peer.did);
        this._advanceHead(peer.ref, next);
      }
    }
  }

  async _publishAndApply(cbor, next) {
    const ref = await this.publish({
      tableRef: this.tableRef,
      prevRef: this.head.prevRef,
      seq: this.head.seq,
      tableTid: this.tableTid,
      actionCbor: cbor,
    });
    // Apply our own action so our state matches every peer's after this slot.
    this.agent.receive_action(cbor, this.did);
    this._advanceHead(ref, next);
  }

  /** Record the applied action and move the shared head to the next slot. */
  _advanceHead(ref, next) {
    this.chain.push({ seq: this.head.seq, seat: next.seat, kind: next.kind, ref });
    this.head = { prevRef: ref, seq: this.head.seq + 1 };
    // Drop any stale buffered echoes we've now passed.
    for (const seq of this._pending.keys()) {
      if (seq < this.head.seq) this._pending.delete(seq);
    }
  }

  // ─── Observable state ───────────────────────────────────────────────

  get isComplete() {
    return this._complete;
  }

  get needsBet() {
    return this._needsBet;
  }

  get betOptions() {
    return this._betOptions;
  }

  /** The next global seq the chain expects (also the count applied so far). */
  get nextSeq() {
    return this.head.seq;
  }

  /** Firehose actions buffered out of order, awaiting their global slot. */
  get pendingCount() {
    return this._pending.size;
  }

  /** The strong ref of the last applied action (the current chain tip), or null. */
  get tipRef() {
    return this.head.prevRef;
  }
}
