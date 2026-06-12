//! WASM bindings for the poker and blackjack agents.
//!
//! Thin wasm-bindgen layer over PlayerAgent and BlackjackAgent. All I/O is
//! byte arrays (DAG-CBOR).

use wasm_bindgen::prelude::*;

use crate::agent::{AgentOutput, PlayerAgent};
use crate::blackjack::agent::{BjAgentOutput, BlackjackAgent};
use crate::blackjack::game::Decision;
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

    /// JSON list of pending protocol steps and the seats that owe them:
    /// `[{"kind":"shuffleDeck","seats":[1]}, ...]` (revealLockKey entries also
    /// carry a deckPosition). Empty array when nothing is pending.
    pub fn waiting_on(&self) -> String {
        self.inner.waiting_on_json()
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

#[wasm_bindgen]
pub struct WasmBlackjackAgent {
    inner: BlackjackAgent,
}

/// Result from the blackjack agent: actions to emit, an interactive need, or
/// waiting.
#[wasm_bindgen]
pub struct WasmBjOutput {
    /// "actions", "need_wager", "need_insurance", "need_decision", or "waiting"
    kind: String,
    /// CBOR-encoded action payloads (for "actions" kind)
    actions: Vec<Vec<u8>>,
    /// Options as JSON: wager bounds or available decisions
    options: String,
}

#[wasm_bindgen]
impl WasmBjOutput {
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

    /// Options as JSON (for "need_wager" and "need_decision" kinds).
    #[wasm_bindgen(getter)]
    pub fn options(&self) -> String {
        self.options.clone()
    }
}

fn to_wasm_bj_output(output: BjAgentOutput) -> WasmBjOutput {
    match output {
        BjAgentOutput::Actions(actions) => WasmBjOutput {
            kind: "actions".into(),
            actions,
            options: String::new(),
        },
        BjAgentOutput::NeedWager { min, max } => WasmBjOutput {
            kind: "need_wager".into(),
            actions: vec![],
            options: format!(r#"{{"min":{},"max":{}}}"#, min, max),
        },
        BjAgentOutput::NeedInsurance => WasmBjOutput {
            kind: "need_insurance".into(),
            actions: vec![],
            options: String::new(),
        },
        BjAgentOutput::NeedDecision { options } => WasmBjOutput {
            kind: "need_decision".into(),
            actions: vec![],
            options: serde_json::to_string(&options.iter().map(decision_str).collect::<Vec<_>>())
                .unwrap_or_default(),
        },
        BjAgentOutput::Waiting => WasmBjOutput {
            kind: "waiting".into(),
            actions: vec![],
            options: String::new(),
        },
    }
}

#[wasm_bindgen]
impl WasmBlackjackAgent {
    /// Create a new agent with a DID and secret seed.
    #[wasm_bindgen(constructor)]
    pub fn new(did: &str, seed: &[u8]) -> Result<WasmBlackjackAgent, JsValue> {
        let inner =
            BlackjackAgent::new(did, seed).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(WasmBlackjackAgent { inner })
    }

    /// Feed a DAG-CBOR table record. Returns actions to emit.
    pub fn receive_table(&mut self, cbor: &[u8]) -> Result<WasmBjOutput, JsValue> {
        self.inner
            .receive_table(cbor)
            .map(to_wasm_bj_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Feed a DAG-CBOR action from any player.
    pub fn receive_action(&mut self, cbor: &[u8]) -> Result<WasmBjOutput, JsValue> {
        self.inner
            .receive_action(cbor)
            .map(to_wasm_bj_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Submit a player action. One of: "wager:AMOUNT", "insurance:yes",
    /// "insurance:no", "hit", "stand", "double", "split", "surrender".
    pub fn act(&mut self, action: &str) -> Result<WasmBjOutput, JsValue> {
        let parsed = parse_bj_action(action).map_err(|e| JsValue::from_str(&e))?;
        let result = match parsed {
            BjAct::Wager(amount) => self.inner.wager(amount),
            BjAct::Insurance(take) => self.inner.insurance(take),
            BjAct::Decide(decision) => self.inner.decide(decision),
        };
        result
            .map(to_wasm_bj_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Check if we need a wager, insurance answer, or decision.
    pub fn check_status(&mut self) -> Result<WasmBjOutput, JsValue> {
        self.inner
            .auto_respond_if_needed()
            .map(to_wasm_bj_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Advance to the next round (call after the current round is Complete
    /// and the game isn't over). Returns this player's actions for the new
    /// round.
    pub fn next_round(&mut self) -> Result<WasmBjOutput, JsValue> {
        self.inner
            .next_round()
            .map(to_wasm_bj_output)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// This player's hand(s) as JSON (e.g., [["8c","8d"]]; two arrays after a split).
    pub fn my_hands(&self) -> String {
        let hands: Vec<Vec<String>> = self
            .inner
            .my_hands()
            .iter()
            .map(|hand| hand.iter().map(|c| c.to_string()).collect())
            .collect();
        serde_json::to_string(&hands).unwrap_or_default()
    }

    /// The banker's face-up cards as JSON array of strings.
    pub fn banker_cards(&self) -> String {
        let cards: Vec<String> = self
            .inner
            .banker_cards()
            .iter()
            .map(|c| c.to_string())
            .collect();
        serde_json::to_string(&cards).unwrap_or_default()
    }

    /// Get the current protocol phase.
    /// Returns: "Init", "CommitSeeds", "Shuffle", "Lock", "Wagering",
    /// "Dealing", "Insurance", "PlayerTurn", "Complete"
    pub fn phase(&self) -> String {
        format!("{:?}", self.inner.phase())
    }

    /// Get game state as JSON: minBet, banker, bankerCards, actionOn, players.
    pub fn game_state(&self) -> String {
        self.inner.game_state_json()
    }

    /// JSON of the most recently completed round's result, or "" if none yet.
    pub fn last_round_result(&self) -> String {
        self.inner.last_round_result_json().unwrap_or_default()
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

/// A parsed blackjack player action: wager, insurance answer, or decision.
enum BjAct {
    Wager(u64),
    Insurance(bool),
    Decide(Decision),
}

fn parse_bj_action(s: &str) -> Result<BjAct, String> {
    match s {
        "hit" => Ok(BjAct::Decide(Decision::Hit)),
        "stand" => Ok(BjAct::Decide(Decision::Stand)),
        "double" => Ok(BjAct::Decide(Decision::Double)),
        "split" => Ok(BjAct::Decide(Decision::Split)),
        "surrender" => Ok(BjAct::Decide(Decision::Surrender)),
        "insurance:yes" => Ok(BjAct::Insurance(true)),
        "insurance:no" => Ok(BjAct::Insurance(false)),
        s if s.starts_with("wager:") => {
            let amount: u64 = s[6..].parse().map_err(|_| "invalid wager amount")?;
            Ok(BjAct::Wager(amount))
        }
        _ => Err(format!("unknown blackjack action: {}", s)),
    }
}

fn decision_str(decision: &Decision) -> &'static str {
    match decision {
        Decision::Hit => "hit",
        Decision::Stand => "stand",
        Decision::Double => "double",
        Decision::Split => "split",
        Decision::Surrender => "surrender",
    }
}
