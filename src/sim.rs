//! Game simulator: dummy bus that connects N agents and auto-plays.
//!
//! Creates agents, runs the protocol, broadcasts actions, and records
//! a stream of high-level events suitable for frontend rendering.

use crate::agent::{AgentOutput, PlayerAgent};
use crate::card::Card;
use crate::crypto::{self, Point};
use crate::eval;
use crate::game::BetAction;
use crate::lexicon::re_cardco::poker::table::Table as LexTable;
use crate::protocol::Phase;
use jacquard_common::types::string::Datetime;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A high-level game event for frontend rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GameEvent {
    /// Game started with these players.
    TableCreated {
        players: Vec<String>,
        starting_chips: u64,
        small_blind: u64,
    },
    /// Cryptographic setup phase (commit/shuffle/lock).
    SetupProgress {
        phase: String,
        player: usize,
    },
    /// Blinds posted.
    BlindsPosted {
        small_blind_player: usize,
        small_blind_amount: u64,
        big_blind_player: usize,
        big_blind_amount: u64,
    },
    /// Hole cards dealt to a player (only the player sees their own).
    HoleCardsDealt {
        player: usize,
        cards: Vec<String>,
    },
    /// Community cards revealed.
    CommunityDealt {
        street: String,
        cards: Vec<String>,
    },
    /// A player bet.
    PlayerBet {
        player: usize,
        action: String,
        amount: Option<u64>,
        pot: u64,
    },
    /// A player folded.
    PlayerFolded {
        player: usize,
    },
    /// Showdown: player reveals cards.
    ShowdownReveal {
        player: usize,
        cards: Vec<String>,
        hand_description: String,
    },
    /// Winner announced.
    Winner {
        players: Vec<usize>,
        amount: u64,
        hand_description: String,
    },
    /// Win by fold (everyone else folded).
    WinByFold {
        player: usize,
        amount: u64,
    },
    /// Seeds revealed for verification.
    SeedsVerified,
    /// Game complete with final chip counts.
    GameOver {
        chips: Vec<u64>,
    },
}

/// Bot strategy for auto-play.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BotStrategy {
    /// Always check or call.
    Passive,
    /// Random: mix of check/call/raise/fold.
    Random,
}

/// Configuration for a simulated game.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimConfig {
    pub num_players: usize,
    pub starting_chips: u64,
    pub small_blind: u64,
    pub strategy: BotStrategy,
    /// Random seed for bot decisions (not the crypto seeds).
    pub rng_seed: u64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            num_players: 2,
            starting_chips: 1000,
            small_blind: 10,
            strategy: BotStrategy::Passive,
            rng_seed: 42,
        }
    }
}

/// Runs a simulated game and produces a stream of events.
pub struct Simulator {
    agents: Vec<PlayerAgent>,
    dids: Vec<String>,
    config: SimConfig,
    events: Vec<GameEvent>,
    /// CBOR actions queued for each agent (indexed by agent, contains actions from others).
    queues: Vec<Vec<Vec<u8>>>,
    card_map: HashMap<Point, Card>,
    rng: u64,
    done: bool,
}

impl Simulator {
    pub fn new(config: SimConfig) -> crate::Result<Self> {
        crypto::init()?;
        let n = config.num_players;
        let mut agents = Vec::with_capacity(n);
        let mut dids = Vec::with_capacity(n);

        for i in 0..n {
            let did = format!("did:plc:player{}", i);
            let seed = format!("sim_seed_{}_{}", i, config.rng_seed).into_bytes();
            agents.push(PlayerAgent::new(&did, &seed)?);
            dids.push(did);
        }

        let card_map = crypto::card_points()?
            .into_iter()
            .map(|(c, p)| (p, c))
            .collect();

        Ok(Self {
            agents,
            dids,
            events: Vec::new(),
            queues: vec![vec![]; n],
            card_map,
            rng: config.rng_seed,
            done: false,
            config,
        })
    }

    /// Run the entire game to completion. Returns all events.
    pub fn run(&mut self) -> crate::Result<&[GameEvent]> {
        self.setup()?;
        self.relay_until_betting()?;
        self.emit_deal_events();

        loop {
            if self.done {
                break;
            }
            self.play_one_bet_round()?;
        }

        self.events.push(GameEvent::GameOver {
            chips: self.agents.iter().enumerate().map(|(i, _)| {
                self.agents[i].phase(); // just to access state
                0 // TODO: expose chip counts from agent
            }).collect(),
        });

        Ok(&self.events)
    }

    /// Get events produced so far.
    pub fn events(&self) -> &[GameEvent] {
        &self.events
    }

    // --- Internal ---

    fn setup(&mut self) -> crate::Result<()> {
        let n = self.config.num_players;

        self.events.push(GameEvent::TableCreated {
            players: self.dids.clone(),
            starting_chips: self.config.starting_chips,
            small_blind: self.config.small_blind,
        });

        // Create table CBOR
        let table = LexTable {
            players: self.dids.iter().map(|d| d.clone().into()).collect(),
            starting_chips: self.config.starting_chips as i64,
            small_blind: self.config.small_blind as i64,
            created_at: Datetime::now(),
            extra_data: None,
        };
        let table_cbor = dasl::drisl::to_vec(&table)
            .map_err(|e| crate::Error::Protocol(format!("table CBOR: {}", e)))?;

        // All agents receive table → emit commitSeed
        for i in 0..n {
            let out = self.agents[i].receive_table(&table_cbor)?;
            self.broadcast_from(i, out);
            self.events.push(GameEvent::SetupProgress {
                phase: "commitSeed".into(),
                player: i,
            });
        }

        // Drain commit queues
        self.drain_queues()?;

        Ok(())
    }

    fn relay_until_betting(&mut self) -> crate::Result<()> {
        for _ in 0..5000 {
            let mut progress = false;

            // Auto-respond for each agent
            for i in 0..self.agents.len() {
                match self.agents[i].auto_respond_if_needed()? {
                    AgentOutput::Actions(actions) if !actions.is_empty() => {
                        progress = true;
                        self.broadcast_actions(i, &actions);
                    }
                    AgentOutput::NeedBet { .. } => return Ok(()),
                    _ => {}
                }
            }

            // Drain message queues
            if self.drain_queues()? {
                progress = true;
            }

            // Check if done
            if self.all_complete() {
                self.done = true;
                return Ok(());
            }

            if !progress {
                // Check for bets
                for i in 0..self.agents.len() {
                    if let AgentOutput::NeedBet { .. } =
                        self.agents[i].auto_respond_if_needed()?
                    {
                        return Ok(());
                    }
                }
                return Ok(());
            }
        }
        Err(crate::Error::Protocol("relay exceeded max iterations".into()))
    }

    fn drain_queues(&mut self) -> crate::Result<bool> {
        let mut any_progress = false;
        for _ in 0..1000 {
            let mut progress = false;
            for i in 0..self.agents.len() {
                let queue = std::mem::take(&mut self.queues[i]);
                for action in &queue {
                    match self.agents[i].receive_action(action)? {
                        AgentOutput::Actions(responses) => {
                            self.broadcast_actions(i, &responses);
                            progress = true;
                        }
                        _ => {}
                    }
                }
                if !queue.is_empty() {
                    progress = true;
                }
            }
            if !progress {
                break;
            }
            any_progress = true;
        }
        Ok(any_progress)
    }

    fn play_one_bet_round(&mut self) -> crate::Result<()> {
        // Find who needs to bet and auto-play
        for _ in 0..100 {
            if self.done {
                return Ok(());
            }

            let mut found_bet = false;
            for i in 0..self.agents.len() {
                if let AgentOutput::NeedBet { options } =
                    self.agents[i].auto_respond_if_needed()?
                {
                    let bet = self.pick_bet(&options);
                    let bet_str = format_bet(&bet);
                    let amount = match &bet {
                        BetAction::Raise(a) => Some(*a),
                        _ => None,
                    };

                    match self.agents[i].bet(bet)? {
                        AgentOutput::Actions(actions) => {
                            match &bet_str[..] {
                                "fold" => self.events.push(GameEvent::PlayerFolded { player: i }),
                                _ => self.events.push(GameEvent::PlayerBet {
                                    player: i,
                                    action: bet_str,
                                    amount,
                                    pot: 0, // TODO: expose pot from agent
                                }),
                            }
                            self.broadcast_actions(i, &actions);
                        }
                        _ => {}
                    }
                    found_bet = true;
                    self.drain_queues()?;
                    break;
                }
            }

            if !found_bet {
                // No bets needed — relay and check for next phase
                self.relay_until_betting()?;
                self.emit_deal_events();

                if self.done || self.all_complete() {
                    self.finish_game()?;
                    return Ok(());
                }

                // Check if still no bets
                let any_bets = (0..self.agents.len()).any(|i| {
                    matches!(
                        self.agents[i].auto_respond_if_needed().ok(),
                        Some(AgentOutput::NeedBet { .. })
                    )
                });
                if !any_bets {
                    self.finish_game()?;
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn finish_game(&mut self) -> crate::Result<()> {
        self.done = true;

        // Emit showdown events for players who have full hands
        for i in 0..self.agents.len() {
            let hole = self.agents[i].hole_cards();
            let community = self.agents[i].community_cards();
            if hole.len() == 2 && community.len() == 5 {
                let mut all_cards = hole.clone();
                all_cards.extend_from_slice(&community);
                let hand = eval::best_hand(&all_cards);
                self.events.push(GameEvent::ShowdownReveal {
                    player: i,
                    cards: hole.iter().map(|c| c.to_string()).collect(),
                    hand_description: hand.to_string(),
                });
            }
        }

        // Run seed verification — agents auto-emit VerifySeed when in Complete phase
        for _ in 0..100 {
            let mut progress = false;
            for i in 0..self.agents.len() {
                if let AgentOutput::Actions(actions) = self.agents[i].auto_respond_if_needed()? {
                    if !actions.is_empty() {
                        self.broadcast_actions(i, &actions);
                        progress = true;
                    }
                }
            }
            if self.drain_queues()? {
                progress = true;
            }
            if !progress {
                break;
            }
        }

        self.events.push(GameEvent::SeedsVerified);
        Ok(())
    }

    fn emit_deal_events(&mut self) {
        // Check for new community cards
        if self.agents.is_empty() {
            return;
        }
        let community = self.agents[0].community_cards();
        let community_strs: Vec<String> = community.iter().map(|c| c.to_string()).collect();

        // Emit hole card events
        for i in 0..self.agents.len() {
            let hole = self.agents[i].hole_cards();
            if hole.len() == 2 {
                // Only emit once — check if we already emitted
                let already = self.events.iter().any(|e| matches!(e, GameEvent::HoleCardsDealt { player, .. } if *player == i));
                if !already {
                    self.events.push(GameEvent::HoleCardsDealt {
                        player: i,
                        cards: hole.iter().map(|c| c.to_string()).collect(),
                    });
                }
            }
        }

        // Community card events
        let prev_community_count = self.events.iter().filter(|e| matches!(e, GameEvent::CommunityDealt { .. })).count();
        match (prev_community_count, community.len()) {
            (0, n) if n >= 3 => {
                self.events.push(GameEvent::CommunityDealt {
                    street: "flop".into(),
                    cards: community_strs[..3].to_vec(),
                });
                if n >= 4 {
                    self.events.push(GameEvent::CommunityDealt {
                        street: "turn".into(),
                        cards: vec![community_strs[3].clone()],
                    });
                }
                if n >= 5 {
                    self.events.push(GameEvent::CommunityDealt {
                        street: "river".into(),
                        cards: vec![community_strs[4].clone()],
                    });
                }
            }
            (1, n) if n >= 4 => {
                self.events.push(GameEvent::CommunityDealt {
                    street: "turn".into(),
                    cards: vec![community_strs[3].clone()],
                });
                if n >= 5 {
                    self.events.push(GameEvent::CommunityDealt {
                        street: "river".into(),
                        cards: vec![community_strs[4].clone()],
                    });
                }
            }
            (2, n) if n >= 5 => {
                self.events.push(GameEvent::CommunityDealt {
                    street: "river".into(),
                    cards: vec![community_strs[4].clone()],
                });
            }
            _ => {}
        }
    }

    fn next_rng(&mut self) -> u64 {
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.rng >> 33
    }

    fn pick_bet(&mut self, options: &[BetAction]) -> BetAction {
        match self.config.strategy {
            BotStrategy::Passive => {
                if options.iter().any(|o| matches!(o, BetAction::Check)) {
                    BetAction::Check
                } else {
                    BetAction::Call
                }
            }
            BotStrategy::Random => {
                // Weighted random: prefer action over folding
                // ~40% check/call, ~30% raise, ~20% all-in, ~10% fold
                let roll = self.next_rng() % 100;
                let has_check = options.iter().any(|o| matches!(o, BetAction::Check));
                let has_call = options.iter().any(|o| matches!(o, BetAction::Call));
                let has_raise = options.iter().any(|o| matches!(o, BetAction::Raise(_)));
                let has_allin = options.iter().any(|o| matches!(o, BetAction::AllIn));

                if roll < 40 {
                    // Check or call
                    if has_check { BetAction::Check }
                    else if has_call { BetAction::Call }
                    else { options[0].clone() }
                } else if roll < 70 && has_raise {
                    // Raise — pick the raise option
                    options.iter().find(|o| matches!(o, BetAction::Raise(_))).unwrap().clone()
                } else if roll < 90 && has_allin {
                    BetAction::AllIn
                } else if roll >= 90 {
                    // Fold (but not if we can check for free)
                    if has_check { BetAction::Check }
                    else { BetAction::Fold }
                } else {
                    // Fallback
                    if has_check { BetAction::Check }
                    else if has_call { BetAction::Call }
                    else { options[0].clone() }
                }
            }
        }
    }

    fn broadcast_from(&mut self, from: usize, output: AgentOutput) {
        if let AgentOutput::Actions(actions) = output {
            self.broadcast_actions(from, &actions);
        }
    }

    fn broadcast_actions(&mut self, from: usize, actions: &[Vec<u8>]) {
        for i in 0..self.agents.len() {
            if i != from {
                self.queues[i].extend(actions.iter().cloned());
            }
        }
    }

    fn all_complete(&self) -> bool {
        self.agents.iter().all(|a| matches!(a.phase(), Phase::Complete))
    }
}

fn format_bet(bet: &BetAction) -> String {
    match bet {
        BetAction::Fold => "fold".into(),
        BetAction::Check => "check".into(),
        BetAction::Call => "call".into(),
        BetAction::AllIn => "allIn".into(),
        BetAction::Raise(a) => format!("raise:{}", a),
    }
}
