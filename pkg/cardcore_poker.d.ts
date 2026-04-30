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
     * Get game state as JSON: pot, chips, bets, actionOn, players.
     */
    game_state(): string;
    /**
     * Get hole cards as JSON array of strings (e.g., ["As", "Kh"]).
     */
    hole_cards(): string;
    /**
     * Create a new agent with a DID and secret seed.
     */
    constructor(did: string, seed: Uint8Array);
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
    readonly __wbg_wasmoutput_free: (a: number, b: number) => void;
    readonly simulate_game: (a: number, b: bigint, c: bigint, d: number, e: number, f: bigint) => [number, number, number, number];
    readonly wasmagent_bet: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmagent_check_status: (a: number) => [number, number, number];
    readonly wasmagent_community_cards: (a: number) => [number, number];
    readonly wasmagent_game_state: (a: number) => [number, number];
    readonly wasmagent_hole_cards: (a: number) => [number, number];
    readonly wasmagent_new: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly wasmagent_phase: (a: number) => [number, number];
    readonly wasmagent_receive_action: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmagent_receive_table: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmoutput_action: (a: number, b: number) => [number, number];
    readonly wasmoutput_action_count: (a: number) => number;
    readonly wasmoutput_bet_options: (a: number) => [number, number];
    readonly wasmoutput_kind: (a: number) => [number, number];
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
