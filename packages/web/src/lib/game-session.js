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
  }

  async receiveTable(tableCbor) {
    const out = this.agent.receive_table(tableCbor);
    await this._processOutput(out);
  }

  async receiveAction(actionCbor) {
    const out = this.agent.receive_action(actionCbor);
    await this._processOutput(out);
  }

  async bet(action) {
    const out = this.agent.bet(action);
    await this._processOutput(out);
  }

  async _processOutput(output) {
    if (output.kind === "actions") {
      // Serialize publishes so putRecord calls land on the PDS in seq order.
      for (let i = 0; i < output.action_count; i++) {
        const cbor = new Uint8Array(output.action(i));
        const mySeq = this.seq++;
        this.publishing = this.publishing.then(() => this.publishAction({ seq: mySeq, cbor }));
      }
      await this.publishing;
      this._needsBet = false;
      this._betOptions = [];
    } else if (output.kind === "need_bet") {
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
