/**
 * Paid-hand controller — drives one Cardcore seat through the atbloons wallet
 * handoff for exactly one poker hand.
 *
 * One controller belongs to one signed-in DID and one hand. The host seat
 * proposes, funds itself, activates, and settles. A non-host seat funds and
 * may withdraw before activation. Each step opens the confidential wallet with
 * a full-page redirect; the wallet returns a public receipt in the URL
 * fragment. The controller never touches an OAuth token or a DPoP key.
 *
 * State survives a full-page redirect through an injectable key-value store
 * (browser `localStorage` by default). Every step generates a fresh opaque
 * `state`; the controller rejects a receipt whose `state` does not echo the
 * pending step, so a stray or forged fragment cannot advance the hand.
 */

import {
  COMMAND_KIND,
  CONTRACT_COLLECTION,
  RECEIPT_STATUS,
  buildHandoffUrl,
  newHandoffState,
  publishedReference,
  readReceiptFragment,
  HandoffError,
} from "./handoff.js";

/** Grant lifecycle states, mirrored from the wallet grant model. */
export const GRANT_STATE = Object.freeze({
  IDLE: "idle",
  ACTIVE: "active",
  SETTLED: "settled",
  WITHDRAWN: "withdrawn",
  REVOKED: "revoked",
});

export const SEAT_ROLE = Object.freeze({
  HOST: "host",
  PARTICIPANT: "participant",
});

const STEP_STATUS = Object.freeze({
  IDLE: "idle",
  PENDING: "pending",
  COMPLETE: "complete",
  RETRYABLE: "retryable",
  FAILED: "failed",
});

const TERMINAL_KINDS = new Set([COMMAND_KIND.WITHDRAW, COMMAND_KIND.SETTLE]);

const EMPTY_STATE = Object.freeze({
  role: null,
  table: null,
  soulsPerChip: null,
  contract: null,
  continuation: null,
  grant: GRANT_STATE.IDLE,
  step: STEP_STATUS.IDLE,
  pendingKind: null,
  pendingState: null,
  lastReceipt: null,
  lastError: null,
  history: [],
});

/** In-memory store used when no browser `localStorage` is present. */
class MemoryStore {
  constructor() {
    this._map = new Map();
  }
  getItem(key) {
    return this._map.has(key) ? this._map.get(key) : null;
  }
  setItem(key, value) {
    this._map.set(key, String(value));
  }
  removeItem(key) {
    this._map.delete(key);
  }
}

export class PaidHandController {
  /**
   * @param {object} opts
   * @param {string} opts.walletBaseUrl - operator wallet public origin
   * @param {{networkId:string, protocolVersion:string, genesisHash:string}} opts.scope
   * @param {string} opts.returnUrl - HTTPS or exact loopback game origin
   * @param {string} [opts.storageKey] - persistence key; defaults per return origin
   * @param {{getItem:Function,setItem:Function,removeItem:Function}} [opts.storage]
   * @param {() => string} [opts.newState] - state generator, for deterministic tests
   */
  constructor({ walletBaseUrl, scope, returnUrl, storageKey, storage, newState } = {}) {
    if (!walletBaseUrl) throw new HandoffError("walletBaseUrl is required");
    if (!scope) throw new HandoffError("scope is required");
    if (!returnUrl) throw new HandoffError("returnUrl is required");
    this.walletBaseUrl = walletBaseUrl;
    this.scope = scope;
    this.returnUrl = returnUrl;
    this.storageKey = storageKey || `atbloons.paid-hand.${returnUrl}`;
    this.storage = storage || defaultStorage();
    this._newState = newState || null;
    this.state = this._load();
  }

  // ─── persistence ─────────────────────────────────────────────────

  _load() {
    try {
      const raw = this.storage.getItem(this.storageKey);
      if (!raw) return { ...EMPTY_STATE, history: [] };
      const parsed = JSON.parse(raw);
      return { ...EMPTY_STATE, ...parsed, history: parsed.history || [] };
    } catch {
      return { ...EMPTY_STATE, history: [] };
    }
  }

  _save() {
    this.storage.setItem(this.storageKey, JSON.stringify(this.state));
  }

  /** Forget this hand entirely. Use only after the grant reaches an end state. */
  reset() {
    this.state = { ...EMPTY_STATE, history: [] };
    try {
      this.storage.removeItem(this.storageKey);
    } catch {
      this.storage.setItem(this.storageKey, JSON.stringify(this.state));
    }
  }

  // ─── read-only views ─────────────────────────────────────────────

  get grant() {
    return this.state.grant;
  }
  get contract() {
    return this.state.contract;
  }
  get isClosed() {
    return (
      this.state.grant === GRANT_STATE.SETTLED ||
      this.state.grant === GRANT_STATE.WITHDRAWN ||
      this.state.grant === GRANT_STATE.REVOKED
    );
  }
  get canActivate() {
    return (
      this.state.role === SEAT_ROLE.HOST &&
      this.state.grant === GRANT_STATE.ACTIVE &&
      !!this.state.contract
    );
  }
  get canWithdraw() {
    return (
      this.state.grant === GRANT_STATE.ACTIVE && !!this.state.contract && !!this.state.continuation
    );
  }
  get canSettle() {
    return (
      this.state.role === SEAT_ROLE.HOST &&
      this.state.grant === GRANT_STATE.ACTIVE &&
      !!this.state.contract
    );
  }

  // ─── step starters (return a full-page wallet URL) ───────────────

  /**
   * Host: propose the contract for a finalized table. Opens the wallet review.
   * @returns {string} the wallet handoff URL to navigate to
   */
  startProposal({ table, soulsPerChip }) {
    this._requireOpen();
    const state = this._nextState();
    const url = buildHandoffUrl(this.walletBaseUrl, {
      kind: COMMAND_KIND.PROPOSE,
      scope: this.scope,
      returnUrl: this.returnUrl,
      state,
      table,
      soulsPerChip,
    });
    this._beginStep({
      role: SEAT_ROLE.HOST,
      kind: COMMAND_KIND.PROPOSE,
      state,
      table,
      soulsPerChip,
    });
    return url;
  }

  /**
   * Any seat: fund a known contract. The host funds itself as part of PROPOSE,
   * so a host uses this only when it did not fund during proposal.
   * @returns {string}
   */
  startFunding({ contract }) {
    this._requireOpen();
    const state = this._nextState();
    const role = this.state.role || SEAT_ROLE.PARTICIPANT;
    const url = buildHandoffUrl(this.walletBaseUrl, {
      kind: COMMAND_KIND.FUND,
      scope: this.scope,
      returnUrl: this.returnUrl,
      state,
      contract,
    });
    this._beginStep({ role, kind: COMMAND_KIND.FUND, state, contract });
    return url;
  }

  /** Host: activate after every funding is confirmed. Continues the grant. */
  startActivation() {
    if (!this.canActivate)
      throw new HandoffError("Activation requires a host grant with a contract");
    return this._startContinuation(COMMAND_KIND.ACTIVATE, { contract: this.state.contract });
  }

  /** Any funded seat: withdraw before activation. Terminal for that grant. */
  startWithdrawal() {
    if (!this.canWithdraw) throw new HandoffError("Withdrawal requires an active funded grant");
    return this._startContinuation(COMMAND_KIND.WITHDRAW, { contract: this.state.contract });
  }

  /** Host: settle after a terminal Cardcore action. Terminal for the grant. */
  startSettlement({ terminalAction }) {
    if (!this.canSettle) throw new HandoffError("Settlement requires a host grant with a contract");
    return this._startContinuation(COMMAND_KIND.SETTLE, {
      contract: this.state.contract,
      terminalAction,
    });
  }

  _startContinuation(kind, extra) {
    if (!this.state.continuation)
      throw new HandoffError("A continuation token is required for this step");
    const state = this._nextState();
    const url = buildHandoffUrl(
      this.walletBaseUrl,
      { kind, scope: this.scope, returnUrl: this.returnUrl, state, ...extra },
      this.state.continuation,
    );
    this._beginStep({ role: this.state.role, kind, state });
    return url;
  }

  // ─── receipt handling ────────────────────────────────────────────

  /**
   * Read the return fragment, apply the receipt, and strip the fragment from
   * browser history when a window is present.
   * @param {string} [hash] - defaults to the current window fragment
   * @returns {object|null} the applied receipt, or null when none present
   */
  consumeReturn(hash) {
    const fragment = hash != null ? hash : windowHash();
    const receipt = readReceiptFragment(fragment);
    if (!receipt) return null;
    this.applyReceipt(receipt);
    stripWindowFragment();
    return receipt;
  }

  /**
   * Apply a validated receipt to the hand state. Rejects a receipt whose
   * `state` does not echo the pending step or whose `kind` does not match.
   * @param {object} receipt - a receipt from `readReceiptFragment`/`decodeReceipt`
   * @returns {object} the updated hand state snapshot
   */
  applyReceipt(receipt) {
    if (!receipt || typeof receipt !== "object") throw new HandoffError("receipt is required");
    if (this.state.pendingState == null) throw new HandoffError("No pending handoff step to apply");
    if (receipt.state !== this.state.pendingState)
      throw new HandoffError("receipt state does not match the request");
    if (receipt.kind !== this.state.pendingKind)
      throw new HandoffError("receipt kind does not match the request");

    this.state.lastReceipt = receipt;
    this.state.history = [...this.state.history, { kind: receipt.kind, status: receipt.status }];

    switch (receipt.status) {
      case RECEIPT_STATUS.COMPLETE:
        this._applyComplete(receipt);
        break;
      case RECEIPT_STATUS.RETRYABLE:
        this.state.step = STEP_STATUS.RETRYABLE;
        this.state.lastError = receipt.errorCode;
        // Keep pendingKind/pendingState so a retry reuses the same intent.
        this._save();
        return this.snapshot();
      case RECEIPT_STATUS.FAILED:
        this._applyFailed(receipt);
        break;
      default:
        throw new HandoffError("receipt status is invalid");
    }
    this.state.pendingKind = null;
    this.state.pendingState = null;
    this._save();
    return this.snapshot();
  }

  _applyComplete(receipt) {
    this.state.step = STEP_STATUS.COMPLETE;
    this.state.lastError = null;
    if (receipt.kind === COMMAND_KIND.PROPOSE) {
      const contract = publishedReference(receipt, CONTRACT_COLLECTION.CONTRACT);
      if (contract) this.state.contract = contract;
    }
    if (TERMINAL_KINDS.has(receipt.kind)) {
      this.state.grant =
        receipt.kind === COMMAND_KIND.SETTLE ? GRANT_STATE.SETTLED : GRANT_STATE.WITHDRAWN;
      this.state.continuation = null;
    } else {
      this.state.grant = GRANT_STATE.ACTIVE;
      this.state.continuation = receipt.continuation || this.state.continuation;
    }
  }

  _applyFailed(receipt) {
    this.state.step = STEP_STATUS.FAILED;
    this.state.lastError = receipt.errorCode;
    // A contested or invalid contract revokes the grant. Other terminal
    // failures (bad evidence, insufficient funds) leave the grant open so the
    // seat can correct the input and retry from the same role.
    if (receipt.errorCode === "contract_terminal") {
      this.state.grant = GRANT_STATE.REVOKED;
      this.state.continuation = null;
    }
  }

  snapshot() {
    return {
      role: this.state.role,
      grant: this.state.grant,
      step: this.state.step,
      contract: this.state.contract,
      soulsPerChip: this.state.soulsPerChip,
      table: this.state.table,
      lastError: this.state.lastError,
      canActivate: this.canActivate,
      canWithdraw: this.canWithdraw,
      canSettle: this.canSettle,
      isClosed: this.isClosed,
    };
  }

  // ─── internals ───────────────────────────────────────────────────

  _requireOpen() {
    if (this.isClosed) throw new HandoffError("This hand grant is already closed");
  }

  _nextState() {
    // A dedicated generator lets tests be deterministic. Production uses the
    // handoff module's CSPRNG-backed 128-bit state.
    return this._newState ? this._newState() : newHandoffState();
  }

  _beginStep({ role, kind, state, table, soulsPerChip, contract }) {
    if (role) this.state.role = role;
    if (table !== undefined) this.state.table = table;
    if (soulsPerChip !== undefined) this.state.soulsPerChip = soulsPerChip;
    if (contract !== undefined) this.state.contract = contract;
    this.state.pendingKind = kind;
    this.state.pendingState = state;
    this.state.step = STEP_STATUS.PENDING;
    if (this.state.grant === GRANT_STATE.IDLE) this.state.grant = GRANT_STATE.ACTIVE;
    this._save();
  }
}

// ─── environment helpers ───────────────────────────────────────────

function defaultStorage() {
  if (typeof globalThis !== "undefined" && globalThis.localStorage) return globalThis.localStorage;
  return new MemoryStore();
}

function windowHash() {
  if (typeof globalThis !== "undefined" && globalThis.location)
    return globalThis.location.hash || "";
  return "";
}

function stripWindowFragment() {
  const w = typeof globalThis !== "undefined" ? globalThis : null;
  if (!w || !w.location || !w.history || typeof w.history.replaceState !== "function") return;
  const { pathname, search } = w.location;
  w.history.replaceState(null, "", `${pathname}${search}`);
}
