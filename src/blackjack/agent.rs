//! Blackjack player agent: CBOR in, CBOR out.
//!
//! The agent wraps the blackjack protocol state machine and handles all
//! crypto internally. Feed it DAG-CBOR encoded AT Protocol records and it
//! emits response records. Non-interactive actions (commit, shuffle, lock,
//! reveals) happen automatically. It pauses only when a human decision is
//! needed: a wager, an insurance answer, or a playing decision.

use jacquard_common::deps::bytes::Bytes;

use crate::agent_util;
use crate::blackjack::game::Decision;
use crate::blackjack::protocol::{BjAction, BjPhase, BjProtocolState, BjValidActionKind};
use crate::card::Card;
use crate::crypto::{self, PlayerKeys, PlayerRng, Point, Scalar};
use crate::lexicon::re_cardco::blackjack::action::ActionAction;
use crate::lexicon::re_cardco::blackjack::table::Table as LexTable;
use crate::lexicon::re_cardco::blackjack::{
    CommitSeed, Decision as LexDecision, DecisionMove, Insurance, LockDeck, RevealLockKey,
    ShuffleDeck, VerifySeed, Wager,
};

/// What the agent needs from the caller.
#[derive(Debug)]
pub enum BjAgentOutput {
    /// CBOR-encoded action records to publish to this player's AT Protocol repo.
    Actions(Vec<Vec<u8>>),
    /// Agent needs a wager. Call `wager()` with an amount in `[min, max]`.
    NeedWager { min: u64, max: u64 },
    /// Agent needs an insurance answer. Call `insurance()` with the choice.
    NeedInsurance,
    /// Agent needs a playing decision. Call `decide()` with one of `options`.
    NeedDecision { options: Vec<Decision> },
    /// Waiting for other players' actions. Nothing to do yet.
    Waiting,
}

pub struct BlackjackAgent {
    pub did: String,
    /// The long-lived secret seed supplied at construction. Per-round seeds
    /// are derived from it so each round uses fresh randomness while a single
    /// round's seed can still be revealed without compromising the others.
    master_seed: Vec<u8>,
    keys: PlayerKeys,
    state: BjProtocolState,
    seat: Option<usize>,
    seq: i64,
    /// Which round we're playing — must track protocol.hand_index for key
    /// derivation.
    hand_index: u64,
}

impl BlackjackAgent {
    /// Create a new agent with the player's DID and secret seed.
    pub fn new(did: &str, seed: &[u8]) -> crate::Result<Self> {
        crypto::init()?;
        let master_seed = seed.to_vec();
        let phs = agent_util::per_hand_seed(&master_seed, 0)?;
        let mut rng = PlayerRng::new(&phs, b"shuffle")?;
        let keys = PlayerKeys::generate(&mut rng)?;
        Ok(Self {
            did: did.to_string(),
            master_seed,
            keys,
            state: BjProtocolState::new(),
            seat: None,
            seq: 0,
            hand_index: 0,
        })
    }

    /// Seed for the current round, derived from the master seed and round index.
    fn per_hand_seed(&self) -> crate::Result<Vec<u8>> {
        agent_util::per_hand_seed(&self.master_seed, self.hand_index)
    }

    /// Regenerate this round's shuffle/lock keys from the current per-round seed.
    fn rederive_keys(&mut self) -> crate::Result<()> {
        let phs = self.per_hand_seed()?;
        let mut rng = PlayerRng::new(&phs, b"shuffle")?;
        self.keys = PlayerKeys::generate(&mut rng)?;
        Ok(())
    }

    /// Advance to the next round once the current one is Complete. Rotates
    /// the banker, rederives fresh keys, and auto-emits this player's new
    /// CommitSeed.
    pub fn next_round(&mut self) -> crate::Result<BjAgentOutput> {
        if self.state.game_over() {
            return Ok(BjAgentOutput::Waiting);
        }
        self.state.start_next_round();
        self.hand_index = self.state.hand_index;
        self.rederive_keys()?;
        self.auto_respond()
    }

    /// JSON of the most recently completed round's result, if any.
    pub fn last_round_result_json(&self) -> Option<String> {
        self.state
            .last_round_result
            .as_ref()
            .and_then(|r| serde_json::to_string(r).ok())
    }

    /// Whether the whole game is over (at most one player with chips).
    pub fn game_over(&self) -> bool {
        self.state.game_over()
    }

    /// Feed a DAG-CBOR encoded table record. This starts the game.
    pub fn receive_table(&mut self, cbor: &[u8]) -> crate::Result<BjAgentOutput> {
        let table: LexTable = dasl::drisl::from_slice(cbor)
            .map_err(|e| crate::Error::Protocol(format!("invalid table CBOR: {}", e)))?;

        // Find our seat. A DID that isn't in the roster becomes a SPECTATOR:
        // seat stays None, the state machine tracks every action like a
        // player's would (the whole game is public in blackjack), but the
        // agent never emits actions and can't wager or act.
        self.seat = table
            .players
            .iter()
            .position(|did| did.as_str() == self.did);

        let players: Vec<String> = table
            .players
            .iter()
            .map(|d| d.as_str().to_string())
            .collect();

        self.state.apply(&BjAction::Table {
            players,
            starting_chips: table.starting_chips as u64,
            min_bet: table.min_bet as u64,
        })?;

        self.auto_respond()
    }

    /// Feed a DAG-CBOR encoded action payload from any player.
    /// The payload is a map with a `$type` field for dispatch.
    pub fn receive_action(&mut self, cbor: &[u8]) -> crate::Result<BjAgentOutput> {
        let action = decode_action_cbor(cbor)?;
        let internal_action = self.lex_action_to_internal(&action)?;
        self.state.apply(&internal_action)?;
        self.seq += 1;
        self.auto_respond()
    }

    /// Submit this round's wager.
    pub fn wager(&mut self, amount: u64) -> crate::Result<BjAgentOutput> {
        let seat = self.require_seat()?;
        self.state.apply(&BjAction::Wager {
            player_id: seat,
            amount,
        })?;

        let lex = Wager {
            amount: amount as i64,
            extra_data: None,
        };
        let cbor = self.encode_action_union(&ActionAction::Wager(Box::new(lex)))?;
        let mut emitted = vec![cbor];
        self.seq += 1;

        emitted.extend(self.auto_respond_collect()?);
        Ok(BjAgentOutput::Actions(emitted))
    }

    /// Take or decline insurance.
    pub fn insurance(&mut self, take: bool) -> crate::Result<BjAgentOutput> {
        let seat = self.require_seat()?;
        self.state.apply(&BjAction::Insurance {
            player_id: seat,
            take,
        })?;

        let lex = Insurance {
            take,
            extra_data: None,
        };
        let cbor = self.encode_action_union(&ActionAction::Insurance(Box::new(lex)))?;
        let mut emitted = vec![cbor];
        self.seq += 1;

        emitted.extend(self.auto_respond_collect()?);
        Ok(BjAgentOutput::Actions(emitted))
    }

    /// Submit a playing decision for the hand currently in turn.
    pub fn decide(&mut self, decision: Decision) -> crate::Result<BjAgentOutput> {
        let seat = self.require_seat()?;
        self.state.apply(&BjAction::Decision {
            player_id: seat,
            decision,
        })?;

        let lex = LexDecision {
            r#move: decision_to_lex(decision),
            extra_data: None,
        };
        let cbor = self.encode_action_union(&ActionAction::Decision(Box::new(lex)))?;
        let mut emitted = vec![cbor];
        self.seq += 1;

        emitted.extend(self.auto_respond_collect()?);
        Ok(BjAgentOutput::Actions(emitted))
    }

    fn require_seat(&self) -> crate::Result<usize> {
        self.seat
            .ok_or_else(|| crate::Error::Protocol("not seated".into()))
    }

    /// This player's hand(s) this round (two after a split), resolved cards only.
    pub fn my_hands(&self) -> Vec<Vec<Card>> {
        let seat = match self.seat {
            Some(s) => s,
            None => return vec![],
        };
        self.state.game.players[seat]
            .hands
            .iter()
            .map(|h| h.cards.clone())
            .collect()
    }

    /// The banker's face-up cards so far.
    pub fn banker_cards(&self) -> Vec<Card> {
        self.state.game.banker_cards.clone()
    }

    /// Check protocol phase.
    pub fn phase(&self) -> &BjPhase {
        &self.state.phase
    }

    /// Get game state as JSON for the frontend.
    pub fn game_state_json(&self) -> String {
        let game = &self.state.game;
        let (action_on, action_hand): (Option<usize>, Option<usize>) = match &self.state.phase {
            BjPhase::Wagering { action_on } => (Some(*action_on), None),
            BjPhase::Insurance { action_on } => (Some(*action_on), None),
            BjPhase::PlayerTurn { action_on, hand } => (Some(*action_on), Some(*hand)),
            _ => (None, None),
        };
        let players: Vec<serde_json::Value> = game
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let hands: Vec<serde_json::Value> = p
                    .hands
                    .iter()
                    .map(|h| {
                        serde_json::json!({
                            "cards": h.cards.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
                            "total": h.value().total,
                            "soft": h.value().soft,
                            "wager": h.wager,
                            "doubled": h.doubled,
                            "stood": h.stood,
                            "busted": h.busted,
                            "splitAces": h.split_aces,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "seat": i,
                    "chips": p.chips,
                    "wager": p.wager,
                    "insurance": p.insurance,
                    "surrendered": p.surrendered,
                    "eliminated": p.eliminated,
                    "hands": hands,
                })
            })
            .collect();
        serde_json::to_string(&serde_json::json!({
            "minBet": game.min_bet,
            "banker": game.banker,
            "bankerCards": game.banker_cards.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
            "bankerTotal": game.banker_value().total,
            "actionOn": action_on,
            "actionHand": action_hand,
            "handIndex": self.state.hand_index,
            "gameOver": self.state.game_over(),
            "players": players,
        }))
        .unwrap_or_default()
    }

    /// Try to auto-respond if there are pending non-interactive actions.
    pub fn auto_respond_if_needed(&mut self) -> crate::Result<BjAgentOutput> {
        self.auto_respond()
    }

    // --- Internal ---

    /// Process valid actions for this player and emit responses automatically.
    /// Returns the interactive need (wager/insurance/decision) if we hit one.
    fn auto_respond(&mut self) -> crate::Result<BjAgentOutput> {
        let actions = self.auto_respond_collect()?;
        if !actions.is_empty() {
            return Ok(BjAgentOutput::Actions(actions));
        }

        let valid = self.state.valid_actions();
        for va in &valid {
            if va.player_id == self.seat.unwrap_or(usize::MAX) {
                match &va.kind {
                    BjValidActionKind::Wager { min, max } => {
                        return Ok(BjAgentOutput::NeedWager {
                            min: *min,
                            max: *max,
                        });
                    }
                    BjValidActionKind::Insurance => {
                        return Ok(BjAgentOutput::NeedInsurance);
                    }
                    BjValidActionKind::Decision { options } => {
                        return Ok(BjAgentOutput::NeedDecision {
                            options: options.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }

        Ok(BjAgentOutput::Waiting)
    }

    /// Collect all non-interactive actions this player should emit.
    fn auto_respond_collect(&mut self) -> crate::Result<Vec<Vec<u8>>> {
        let mut emitted = Vec::new();
        loop {
            let valid = self.state.valid_actions();
            let seat = match self.seat {
                Some(s) => s,
                None => break,
            };

            // Find a non-interactive action for us (not a wager, insurance,
            // or decision — and not seed verification, which isn't
            // auto-revealed in a multi-round game).
            let my_action = valid.iter().find(|va| {
                va.player_id == seat
                    && matches!(
                        va.kind,
                        BjValidActionKind::CommitSeed
                            | BjValidActionKind::ShuffleDeck
                            | BjValidActionKind::LockDeck
                            | BjValidActionKind::RevealLockKey { .. }
                    )
            });

            let va = match my_action {
                Some(va) => va.clone(),
                None => break,
            };

            match &va.kind {
                BjValidActionKind::CommitSeed => {
                    let commitment = agent_util::seed_commitment(&self.per_hand_seed()?)?;
                    self.state.apply(&BjAction::CommitSeed {
                        player_id: seat,
                        commitment,
                    })?;
                    emitted.push(self.encode_action_union(&ActionAction::CommitSeed(Box::new(
                        CommitSeed {
                            commitment: Bytes::copy_from_slice(&commitment),
                            extra_data: None,
                        },
                    )))?);
                    self.seq += 1;
                }
                BjValidActionKind::ShuffleDeck => {
                    let phs = self.per_hand_seed()?;
                    let encrypted = agent_util::shuffle_deck_response(
                        &self.keys,
                        &phs,
                        &self.state.crypto.deck,
                    )?;

                    let deck_bytes: Vec<Bytes> = encrypted
                        .iter()
                        .map(|p| Bytes::copy_from_slice(&p.0))
                        .collect();

                    self.state.apply(&BjAction::ShuffleDeck {
                        player_id: seat,
                        deck: encrypted,
                    })?;

                    emitted.push(self.encode_action_union(&ActionAction::ShuffleDeck(
                        Box::new(ShuffleDeck {
                            deck: deck_bytes,
                            extra_data: None,
                        }),
                    ))?);
                    self.seq += 1;
                }
                BjValidActionKind::LockDeck => {
                    let phs = self.per_hand_seed()?;
                    let locked = agent_util::lock_deck_response(
                        &mut self.keys,
                        &phs,
                        &self.state.crypto.deck,
                    )?;

                    let deck_bytes: Vec<Bytes> = locked
                        .iter()
                        .map(|p| Bytes::copy_from_slice(&p.0))
                        .collect();

                    self.state.apply(&BjAction::LockDeck {
                        player_id: seat,
                        deck: locked,
                    })?;

                    emitted.push(self.encode_action_union(&ActionAction::LockDeck(Box::new(
                        LockDeck {
                            deck: deck_bytes,
                            extra_data: None,
                        },
                    )))?);
                    self.seq += 1;
                }
                BjValidActionKind::RevealLockKey { deck_position } => {
                    let pos = *deck_position;
                    let scalar = agent_util::reveal_scalar(&self.keys, pos);

                    self.state.apply(&BjAction::RevealLockKey {
                        player_id: seat,
                        deck_position: pos,
                        scalar: scalar.clone(),
                    })?;

                    emitted.push(self.encode_action_union(&ActionAction::RevealLockKey(
                        Box::new(RevealLockKey {
                            deck_position: pos as i64,
                            scalar: Bytes::copy_from_slice(&scalar.0),
                            extra_data: None,
                        }),
                    ))?);
                    self.seq += 1;
                }
                _ => break,
            }
        }
        Ok(emitted)
    }

    /// Encode an ActionAction union variant as DAG-CBOR.
    /// The $type tag is included for dispatch on the receiving end.
    fn encode_action_union(&self, action: &ActionAction<'_>) -> crate::Result<Vec<u8>> {
        dasl::drisl::to_vec(action)
            .map_err(|e| crate::Error::Protocol(format!("CBOR encode failed: {}", e)))
    }

    /// Convert a lexicon action to an internal protocol action.
    fn lex_action_to_internal(&self, action: &ActionAction<'_>) -> crate::Result<BjAction> {
        // Figure out which player this is from based on valid_actions
        let valid = self.state.valid_actions();

        match action {
            ActionAction::CommitSeed(cs) => {
                let player_id =
                    find_player_for_action(&valid, |k| matches!(k, BjValidActionKind::CommitSeed))?;
                let mut commitment = [0u8; crypto::HASH_BYTES];
                commitment.copy_from_slice(&cs.commitment);
                Ok(BjAction::CommitSeed {
                    player_id,
                    commitment,
                })
            }
            ActionAction::ShuffleDeck(sd) => {
                let player_id = find_player_for_action(&valid, |k| {
                    matches!(k, BjValidActionKind::ShuffleDeck)
                })?;
                Ok(BjAction::ShuffleDeck {
                    player_id,
                    deck: bytes_to_points(&sd.deck),
                })
            }
            ActionAction::LockDeck(ld) => {
                let player_id =
                    find_player_for_action(&valid, |k| matches!(k, BjValidActionKind::LockDeck))?;
                Ok(BjAction::LockDeck {
                    player_id,
                    deck: bytes_to_points(&ld.deck),
                })
            }
            ActionAction::RevealLockKey(rlk) => {
                let pos = rlk.deck_position as usize;
                let player_id = find_player_for_action(
                    &valid,
                    |k| matches!(k, BjValidActionKind::RevealLockKey { deck_position } if *deck_position == pos),
                )?;
                let mut scalar_arr = [0u8; crypto::SCALAR_BYTES];
                scalar_arr.copy_from_slice(&rlk.scalar);
                Ok(BjAction::RevealLockKey {
                    player_id,
                    deck_position: pos,
                    scalar: Scalar(scalar_arr),
                })
            }
            ActionAction::Wager(w) => {
                let player_id = find_player_for_action(&valid, |k| {
                    matches!(k, BjValidActionKind::Wager { .. })
                })?;
                Ok(BjAction::Wager {
                    player_id,
                    amount: w.amount as u64,
                })
            }
            ActionAction::Insurance(ins) => {
                let player_id =
                    find_player_for_action(&valid, |k| matches!(k, BjValidActionKind::Insurance))?;
                Ok(BjAction::Insurance {
                    player_id,
                    take: ins.take,
                })
            }
            ActionAction::Decision(d) => {
                let player_id = find_player_for_action(&valid, |k| {
                    matches!(k, BjValidActionKind::Decision { .. })
                })?;
                Ok(BjAction::Decision {
                    player_id,
                    decision: lex_decision_to_internal(&d.r#move)?,
                })
            }
            ActionAction::VerifySeed(vs) => {
                // Match this seed to a player by checking against commitments
                let seed_bytes = vs.seed.to_vec();
                let hash = crypto::blake2b(&seed_bytes)?;
                let player_id = self
                    .state
                    .crypto
                    .seed_commitments
                    .iter()
                    .enumerate()
                    .find(|(i, c)| {
                        c.map_or(false, |commitment| commitment == hash)
                            && !self.state.crypto.seeds_verified[*i]
                    })
                    .map(|(i, _)| i)
                    .ok_or_else(|| {
                        crate::Error::InvalidAction(
                            "seed doesn't match any unverified commitment".into(),
                        )
                    })?;
                Ok(BjAction::VerifySeed {
                    player_id,
                    seed: seed_bytes,
                })
            }
            _ => Err(crate::Error::Protocol("unknown action type".into())),
        }
    }
}

/// Manually decode a DAG-CBOR action payload by reading the $type tag.
/// This is needed because serde's internally-tagged enum doesn't work
/// reliably with DAG-CBOR's sorted map keys.
fn decode_action_cbor<'a>(cbor: &'a [u8]) -> crate::Result<ActionAction<'a>> {
    let value: dasl::drisl::Value = dasl::drisl::from_slice(cbor)
        .map_err(|e| crate::Error::Protocol(format!("invalid CBOR: {}", e)))?;

    let type_tag = match &value {
        dasl::drisl::Value::Map(map) => map
            .get("$type")
            .and_then(|v| match v {
                dasl::drisl::Value::Text(s) => Some(s.as_str()),
                _ => None,
            })
            .ok_or_else(|| crate::Error::Protocol("missing $type field".into()))?,
        _ => return Err(crate::Error::Protocol("expected CBOR map".into())),
    };

    match type_tag {
        "re.cardco.blackjack.defs#commitSeed" => {
            let cs: CommitSeed = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode commitSeed: {}", e)))?;
            Ok(ActionAction::CommitSeed(Box::new(cs)))
        }
        "re.cardco.blackjack.defs#shuffleDeck" => {
            let sd: ShuffleDeck = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode shuffleDeck: {}", e)))?;
            Ok(ActionAction::ShuffleDeck(Box::new(sd)))
        }
        "re.cardco.blackjack.defs#lockDeck" => {
            let ld: LockDeck = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode lockDeck: {}", e)))?;
            Ok(ActionAction::LockDeck(Box::new(ld)))
        }
        "re.cardco.blackjack.defs#revealLockKey" => {
            let rlk: RevealLockKey = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode revealLockKey: {}", e)))?;
            Ok(ActionAction::RevealLockKey(Box::new(rlk)))
        }
        "re.cardco.blackjack.defs#wager" => {
            let w: Wager = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode wager: {}", e)))?;
            Ok(ActionAction::Wager(Box::new(w)))
        }
        "re.cardco.blackjack.defs#insurance" => {
            let ins: Insurance = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode insurance: {}", e)))?;
            Ok(ActionAction::Insurance(Box::new(ins)))
        }
        "re.cardco.blackjack.defs#decision" => {
            let d: LexDecision = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode decision: {}", e)))?;
            Ok(ActionAction::Decision(Box::new(d)))
        }
        "re.cardco.blackjack.defs#verifySeed" => {
            let vs: VerifySeed = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode verifySeed: {}", e)))?;
            Ok(ActionAction::VerifySeed(Box::new(vs)))
        }
        other => Err(crate::Error::Protocol(format!(
            "unknown action type: {}",
            other
        ))),
    }
}

fn bytes_to_points(deck: &[Bytes]) -> Vec<Point> {
    deck.iter()
        .map(|b| {
            let mut arr = [0u8; crypto::POINT_BYTES];
            arr.copy_from_slice(b);
            Point(arr)
        })
        .collect()
}

/// Find which player should be performing an action of this type.
fn find_player_for_action(
    valid: &[crate::blackjack::protocol::BjValidAction],
    predicate: impl Fn(&BjValidActionKind) -> bool,
) -> crate::Result<usize> {
    agent_util::find_player_for_action(valid, |va| predicate(&va.kind), |va| va.player_id)
}

fn decision_to_lex(decision: Decision) -> DecisionMove<'static> {
    match decision {
        Decision::Hit => DecisionMove::Hit,
        Decision::Stand => DecisionMove::Stand,
        Decision::Double => DecisionMove::Double,
        Decision::Split => DecisionMove::Split,
        Decision::Surrender => DecisionMove::Surrender,
    }
}

fn lex_decision_to_internal(mv: &DecisionMove<'_>) -> crate::Result<Decision> {
    match mv {
        DecisionMove::Hit => Ok(Decision::Hit),
        DecisionMove::Stand => Ok(Decision::Stand),
        DecisionMove::Double => Ok(Decision::Double),
        DecisionMove::Split => Ok(Decision::Split),
        DecisionMove::Surrender => Ok(Decision::Surrender),
        DecisionMove::Other(s) => Err(crate::Error::InvalidAction(format!("unknown move: {}", s))),
    }
}
