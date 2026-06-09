//! WASM bindings for the poker agent.
//!
//! Thin wasm-bindgen layer over PlayerAgent. All I/O is byte arrays (DAG-CBOR).

use wasm_bindgen::prelude::*;

use crate::agent::{AgentOutput, PlayerAgent};
use crate::game::BetAction;
use crate::sim::{BotStrategy, SimConfig, Simulator};

#[wasm_bindgen]
pub struct WasmAgent {
    inner: PlayerAgent,
}

/// Result from the agent: either actions to emit, a bet decision needed, or waiting.
#[wasm_bindgen]
pub struct WasmOutput {
    /// "actions", "need_bet", or "waiting"
    kind: String,
    /// CBOR-encoded action payloads (for "actions" kind)
    actions: Vec<Vec<u8>>,
    /// Available bet options as JSON (for "need_bet" kind)
    bet_options: String,
}

#[wasm_bindgen]
impl WasmOutput {
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.clone()
    }

    /// Number of actions to emit.
    #[wasm_bindgen(getter)]
    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    /// Get the nth action as CBOR bytes.
    pub fn action(&self, index: usize) -> Vec<u8> {
        self.actions.get(index).cloned().unwrap_or_default()
    }

    /// Bet options as JSON (only for "need_bet" kind).
    #[wasm_bindgen(getter)]
    pub fn bet_options(&self) -> String {
        self.bet_options.clone()
    }
}

fn to_wasm_output(output: AgentOutput) -> WasmOutput {
    match output {
        AgentOutput::Actions(actions) => WasmOutput {
            kind: "actions".into(),
            actions,
            bet_options: String::new(),
        },
        AgentOutput::NeedBet { options } => WasmOutput {
            kind: "need_bet".into(),
            actions: vec![],
            bet_options: serde_json::to_string(&options).unwrap_or_default(),
        },
        AgentOutput::Waiting => WasmOutput {
            kind: "waiting".into(),
            actions: vec![],
            bet_options: String::new(),
        },
    }
}

#[wasm_bindgen]
impl WasmAgent {
    /// Create a new agent with a DID and secret seed.
    #[wasm_bindgen(constructor)]
    pub fn new(did: &str, seed: &[u8]) -> Result<WasmAgent, JsValue> {
        let inner = PlayerAgent::new(did, seed).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(WasmAgent { inner })
    }

    /// Feed a DAG-CBOR table record. Returns actions to emit.
    pub fn receive_table(&mut self, cbor: &[u8]) -> Result<WasmOutput, JsValue> {
        self.inner
            .receive_table(cbor)
            .map(to_wasm_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Feed a DAG-CBOR action from any player.
    pub fn receive_action(&mut self, cbor: &[u8]) -> Result<WasmOutput, JsValue> {
        self.inner
            .receive_action(cbor)
            .map(to_wasm_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Submit a betting decision. action is one of: "fold", "check", "call", "allIn", or "raise:AMOUNT".
    pub fn bet(&mut self, action: &str) -> Result<WasmOutput, JsValue> {
        let bet = parse_bet_action(action).map_err(|e| JsValue::from_str(&e))?;
        self.inner
            .bet(bet)
            .map(to_wasm_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Get hole cards as JSON array of strings (e.g., ["As", "Kh"]).
    pub fn hole_cards(&self) -> String {
        let cards: Vec<String> = self
            .inner
            .hole_cards()
            .iter()
            .map(|c| c.to_string())
            .collect();
        serde_json::to_string(&cards).unwrap_or_default()
    }

    /// Get community cards as JSON array of strings.
    pub fn community_cards(&self) -> String {
        let cards: Vec<String> = self
            .inner
            .community_cards()
            .iter()
            .map(|c| c.to_string())
            .collect();
        serde_json::to_string(&cards).unwrap_or_default()
    }

    /// Get the current protocol phase.
    /// Returns: "Init", "CommitSeeds", "Shuffle", "Lock", "Dealing", "Betting", "Showdown", "Complete"
    pub fn phase(&self) -> String {
        format!("{:?}", self.inner.phase())
    }

    /// Get game state as JSON: pot, chips, bets, actionOn, players.
    pub fn game_state(&self) -> String {
        self.inner.game_state_json()
    }

    /// Check if we need a bet decision.
    pub fn check_status(&mut self) -> Result<WasmOutput, JsValue> {
        self.inner
            .auto_respond_if_needed()
            .map(to_wasm_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Advance to the next hand (call after the current hand is Complete and the
    /// game isn't over). Returns this player's actions for the new hand.
    pub fn next_hand(&mut self) -> Result<WasmOutput, JsValue> {
        self.inner
            .next_hand()
            .map(to_wasm_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// JSON of the most recently completed hand's result, or "" if none yet.
    pub fn last_hand_result(&self) -> String {
        self.inner.last_hand_result_json().unwrap_or_default()
    }

    /// Whether the whole game is over (at most one player has chips).
    pub fn game_over(&self) -> bool {
        self.inner.game_over()
    }
}

/// Simulate a complete game and return events as JSON.
#[wasm_bindgen]
pub fn simulate_game(
    num_players: usize,
    starting_chips: u64,
    small_blind: u64,
    strategy: &str,
    rng_seed: u64,
) -> Result<String, JsValue> {
    let strategy = match strategy {
        "random" => BotStrategy::Random,
        _ => BotStrategy::Passive,
    };
    let config = SimConfig {
        num_players,
        starting_chips,
        small_blind,
        strategy,
        rng_seed,
    };
    let mut sim = Simulator::new(config).map_err(|e| JsValue::from_str(&e.to_string()))?;
    sim.run().map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_json::to_string(sim.events()).map_err(|e| JsValue::from_str(&e.to_string()))
}

fn parse_bet_action(s: &str) -> Result<BetAction, String> {
    match s {
        "fold" => Ok(BetAction::Fold),
        "check" => Ok(BetAction::Check),
        "call" => Ok(BetAction::Call),
        "allIn" => Ok(BetAction::AllIn),
        s if s.starts_with("raise:") => {
            let amount: u64 = s[6..].parse().map_err(|_| "invalid raise amount")?;
            Ok(BetAction::Raise(amount))
        }
        _ => Err(format!("unknown bet action: {}", s)),
    }
}
