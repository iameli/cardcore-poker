/* tslint:disable */
/* eslint-disable */

export class WasmAgent {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Submit a betting decision. action is one of: "fold", "check", "call", "allIn", or "raise:AMOUNT".
     */
    bet(action: string): WasmOutput;
    /**
     * Check if we need a bet decision.
     */
    check_status(): WasmOutput;
    /**
     * Get community cards as JSON array of strings.
     */
    community_cards(): string;
    /**
     * Whether the whole game is over (at most one player has chips).
     */
    game_over(): boolean;
    /**
     * Get game state as JSON: pot, chips, bets, actionOn, players.
     */
    game_state(): string;
    /**
     * Get hole cards as JSON array of strings (e.g., ["As", "Kh"]).
     */
    hole_cards(): string;
    /**
     * JSON of the most recently completed hand's result, or "" if none yet.
     */
    last_hand_result(): string;
    /**
     * Create a new agent with a DID and secret seed.
     */
    constructor(did: string, seed: Uint8Array);
    /**
     * Advance to the next hand (call after the current hand is Complete and the
     * game isn't over). Returns this player's actions for the new hand.
     */
    next_hand(): WasmOutput;
    /**
     * Get the current protocol phase.
     * Returns: "Init", "CommitSeeds", "Shuffle", "Lock", "Dealing", "Betting", "Showdown", "Complete"
     */
    phase(): string;
    /**
     * Feed a DAG-CBOR action from any player.
     */
    receive_action(cbor: Uint8Array): WasmOutput;
    /**
     * Feed a DAG-CBOR table record. Returns actions to emit.
     */
    receive_table(cbor: Uint8Array): WasmOutput;
    /**
     * JSON list of pending protocol steps and the seats that owe them:
     * `[{"kind":"shuffleDeck","seats":[1]}, ...]` (revealLockKey entries also
     * carry a deckPosition). Empty array when nothing is pending.
     */
    waiting_on(): string;
}

/**
 * Result from the blackjack agent: actions to emit, an interactive need, or
 * waiting.
 */
export class WasmBjOutput {
    /**
     * Number of actions to emit.
     */
    readonly action_count: number;
    readonly kind: string;
    /**
     * Options as JSON (for "need_wager" and "need_decision" kinds).
     */
    readonly options: string;

    private constructor();

    free(): void;

    [Symbol.dispose](): void;

    /**
     * Get the nth action as CBOR bytes.
     */
    action(index: number): Uint8Array;
}

export class WasmBlackjackAgent {
    /**
     * Create a new agent with a DID and secret seed.
     */
    constructor(did: string, seed: Uint8Array);

    free(): void;

    [Symbol.dispose](): void;

    /**
     * Submit a player action. One of: "wager:AMOUNT", "insurance:yes",
     * "insurance:no", "hit", "stand", "double", "split", "surrender".
     */
    act(action: string): WasmBjOutput;

    /**
     * The banker's face-up cards as JSON array of strings.
     */
    banker_cards(): string;

    /**
     * Check if we need a wager, insurance answer, or decision.
     */
    check_status(): WasmBjOutput;

    /**
     * Whether the whole game is over (at most one player has chips).
     */
    game_over(): boolean;

    /**
     * Get game state as JSON: minBet, banker, bankerCards, actionOn, players.
     */
    game_state(): string;

    /**
     * JSON of the most recently completed round's result, or "" if none yet.
     */
    last_round_result(): string;

    /**
     * This player's hand(s) as JSON (e.g., [["8c","8d"]]; two arrays after a split).
     */
    my_hands(): string;

    /**
     * Advance to the next round (call after the current round is Complete
     * and the game isn't over). Returns this player's actions for the new
     * round.
     */
    next_round(): WasmBjOutput;
    /**
     * Get the current protocol phase.
     * Returns: "Init", "CommitSeeds", "Shuffle", "Lock", "Wagering",
     * "Dealing", "Insurance", "PlayerTurn", "Complete"
     */
    phase(): string;
    /**
     * Feed a DAG-CBOR action from any player.
     */
    receive_action(cbor: Uint8Array): WasmBjOutput;
    /**
     * Feed a DAG-CBOR table record. Returns actions to emit.
     */
    receive_table(cbor: Uint8Array): WasmBjOutput;
}

/**
 * Result from the agent: either actions to emit, a bet decision needed, or waiting.
 */
export class WasmOutput {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Get the nth action as CBOR bytes.
     */
    action(index: number): Uint8Array;
    /**
     * Number of actions to emit.
     */
    readonly action_count: number;
    /**
     * Bet options as JSON (only for "need_bet" kind).
     */
    readonly bet_options: string;
    readonly kind: string;
}

/**
 * Simulate a complete game and return events as JSON.
 */
export function simulate_game(num_players: number, starting_chips: bigint, small_blind: bigint, strategy: string, rng_seed: bigint): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_wasmagent_free: (a: number, b: number) => void;
    readonly __wbg_wasmbjoutput_free: (a: number, b: number) => void;
    readonly __wbg_wasmblackjackagent_free: (a: number, b: number) => void;
    readonly simulate_game: (a: number, b: bigint, c: bigint, d: number, e: number, f: bigint) => [number, number, number, number];
    readonly wasmagent_bet: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmagent_check_status: (a: number) => [number, number, number];
    readonly wasmagent_community_cards: (a: number) => [number, number];
    readonly wasmagent_game_over: (a: number) => number;
    readonly wasmagent_game_state: (a: number) => [number, number];
    readonly wasmagent_hole_cards: (a: number) => [number, number];
    readonly wasmagent_last_hand_result: (a: number) => [number, number];
    readonly wasmagent_new: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly wasmagent_next_hand: (a: number) => [number, number, number];
    readonly wasmagent_phase: (a: number) => [number, number];
    readonly wasmagent_receive_action: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmagent_receive_table: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmagent_waiting_on: (a: number) => [number, number];
    readonly wasmbjoutput_action: (a: number, b: number) => [number, number];
    readonly wasmbjoutput_action_count: (a: number) => number;
    readonly wasmbjoutput_kind: (a: number) => [number, number];
    readonly wasmbjoutput_options: (a: number) => [number, number];
    readonly wasmblackjackagent_act: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmblackjackagent_banker_cards: (a: number) => [number, number];
    readonly wasmblackjackagent_check_status: (a: number) => [number, number, number];
    readonly wasmblackjackagent_game_over: (a: number) => number;
    readonly wasmblackjackagent_game_state: (a: number) => [number, number];
    readonly wasmblackjackagent_last_round_result: (a: number) => [number, number];
    readonly wasmblackjackagent_my_hands: (a: number) => [number, number];
    readonly wasmblackjackagent_new: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly wasmblackjackagent_next_round: (a: number) => [number, number, number];
    readonly wasmblackjackagent_phase: (a: number) => [number, number];
    readonly wasmblackjackagent_receive_action: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmblackjackagent_receive_table: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmoutput_action_count: (a: number) => number;
    readonly __wbg_wasmoutput_free: (a: number, b: number) => void;
    readonly wasmoutput_kind: (a: number) => [number, number];
    readonly wasmoutput_action: (a: number, b: number) => [number, number];
    readonly wasmoutput_bet_options: (a: number) => [number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
