//! Player agent: CBOR in, CBOR out.
//!
//! The agent wraps the protocol state machine and handles all crypto internally.
//! Feed it DAG-CBOR encoded AT Protocol records and it emits response records.
//! Non-interactive actions (shuffle, lock, decrypt for others) happen automatically.
//! It pauses only when a human decision is needed (betting).

use jacquard_common::deps::bytes::Bytes;
use rand::prelude::SliceRandom;

use crate::crypto::{self, PlayerKeys, PlayerRng, Point, Scalar};
use crate::game::BetAction;
use crate::lexicon::re_cardco::poker::action::{Action as LexAction, ActionAction};
use crate::lexicon::re_cardco::poker::table::Table as LexTable;
use crate::lexicon::re_cardco::poker::*;
use crate::protocol::{self, Action, Phase, ProtocolState, ValidActionKind};

/// What the agent needs from the caller.
#[derive(Debug)]
pub enum AgentOutput {
    /// CBOR-encoded action records to publish to this player's AT Protocol repo.
    Actions(Vec<Vec<u8>>),
    /// Agent needs a betting decision from the player. Call `bet()` with the choice.
    NeedBet { options: Vec<BetAction> },
    /// Waiting for other players' actions. Nothing to do yet.
    Waiting,
}

pub struct PlayerAgent {
    pub did: String,
    seed: Vec<u8>,
    keys: PlayerKeys,
    state: ProtocolState,
    seat: Option<usize>,
    seq: i64,
    table_tid: Option<String>,
}

impl PlayerAgent {
    /// Create a new agent with the player's DID and secret seed.
    pub fn new(did: &str, seed: &[u8]) -> crate::Result<Self> {
        crypto::init()?;
        let mut rng = PlayerRng::new(seed, b"shuffle")?;
        let keys = PlayerKeys::generate(&mut rng)?;
        Ok(Self {
            did: did.to_string(),
            seed: seed.to_vec(),
            keys,
            state: ProtocolState::new(),
            seat: None,
            seq: 0,
            table_tid: None,
        })
    }

    /// Feed a DAG-CBOR encoded table record. This starts the game.
    pub fn receive_table(&mut self, cbor: &[u8]) -> crate::Result<AgentOutput> {
        let table: LexTable = dasl::drisl::from_slice(cbor)
            .map_err(|e| crate::Error::Protocol(format!("invalid table CBOR: {}", e)))?;

        // Find our seat
        let seat = table
            .players
            .iter()
            .position(|did| did.as_str() == self.did)
            .ok_or_else(|| crate::Error::Protocol("player not at this table".into()))?;
        self.seat = Some(seat);

        let players: Vec<String> = table
            .players
            .iter()
            .map(|d| d.as_str().to_string())
            .collect();

        // Apply table to protocol state
        self.state.apply(&Action::Table {
            players,
            starting_chips: table.starting_chips as u64,
            small_blind: table.small_blind as u64,
        })?;

        // Now auto-respond with any actions we can take
        self.auto_respond()
    }

    /// Feed a DAG-CBOR encoded action payload from any player.
    /// The payload is a map with a `$type` field for dispatch.
    pub fn receive_action(&mut self, cbor: &[u8]) -> crate::Result<AgentOutput> {
        let action = decode_action_cbor(cbor)?;
        let internal_action = self.lex_action_to_internal(&action, self.seq)?;
        self.state.apply(&internal_action)?;
        self.seq += 1;
        self.auto_respond()
    }

    /// Submit a betting decision.
    pub fn bet(&mut self, action: BetAction) -> crate::Result<AgentOutput> {
        let seat = self
            .seat
            .ok_or_else(|| crate::Error::Protocol("not seated".into()))?;

        let amount = match &action {
            BetAction::Raise(amt) => Some(*amt as i64),
            _ => None,
        };
        let lex_action_str = bet_action_to_lex(&action);

        self.state.apply(&Action::Bet {
            player_id: seat,
            action: action.clone(),
        })?;

        let lex_bet = Bet {
            action: lex_action_str,
            amount,
            extra_data: None,
        };

        let cbor = self.encode_action_union(&ActionAction::Bet(Box::new(lex_bet)))?;
        let mut emitted = vec![cbor];
        self.seq += 1;

        let more = self.auto_respond_collect()?;
        emitted.extend(more);
        Ok(AgentOutput::Actions(emitted))
    }

    /// Get this player's resolved hole cards (after dealing).
    pub fn hole_cards(&self) -> Vec<crate::card::Card> {
        let seat = match self.seat {
            Some(s) => s,
            None => return vec![],
        };
        let card_map: std::collections::HashMap<Point, crate::card::Card> = crypto::card_points()
            .unwrap()
            .into_iter()
            .map(|(c, p)| (p, c))
            .collect();

        self.state.game.players[seat]
            .hole_encrypted
            .iter()
            .enumerate()
            .filter_map(|(idx, enc)| {
                let pos = self.state.hole_card_positions[seat].get(idx)?;
                let decrypted = crypto::decrypt(enc, &self.keys.lock_decrypt[*pos]).ok()?;
                card_map.get(&decrypted).copied()
            })
            .collect()
    }

    /// Get the community cards revealed so far.
    pub fn community_cards(&self) -> Vec<crate::card::Card> {
        let card_map: std::collections::HashMap<Point, crate::card::Card> = crypto::card_points()
            .unwrap()
            .into_iter()
            .map(|(c, p)| (p, c))
            .collect();

        self.state
            .game
            .community
            .iter()
            .filter_map(|p| card_map.get(p).copied())
            .collect()
    }

    /// Check protocol phase.
    pub fn phase(&self) -> &Phase {
        &self.state.phase
    }

    /// Get game state as JSON for the frontend.
    pub fn game_state_json(&self) -> String {
        let state = &self.state.game;
        let players: Vec<serde_json::Value> = state
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| {
                serde_json::json!({
                    "seat": i,
                    "chips": p.chips,
                    "bet": p.bet_this_street,
                    "folded": p.folded,
                    "all_in": p.all_in,
                })
            })
            .collect();
        serde_json::to_string(&serde_json::json!({
            "pot": state.pot,
            "currentBet": state.current_bet,
            "actionOn": state.action_on,
            "players": players,
        }))
        .unwrap_or_default()
    }

    /// Try to auto-respond if there are pending non-interactive actions.
    pub fn auto_respond_if_needed(&mut self) -> crate::Result<AgentOutput> {
        self.auto_respond()
    }

    // --- Internal ---

    /// Process valid actions for this player and emit responses automatically.
    /// Returns NeedBet if we hit a betting decision, or the emitted actions.
    fn auto_respond(&mut self) -> crate::Result<AgentOutput> {
        let actions = self.auto_respond_collect()?;
        if !actions.is_empty() {
            return Ok(AgentOutput::Actions(actions));
        }

        // Check if we need a betting decision
        let valid = self.state.valid_actions();
        for va in &valid {
            if va.player_id == self.seat.unwrap_or(usize::MAX) {
                if let ValidActionKind::Bet { options } = &va.kind {
                    return Ok(AgentOutput::NeedBet {
                        options: options.clone(),
                    });
                }
            }
        }

        Ok(AgentOutput::Waiting)
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

            // Find an action for us that isn't a bet or verify-seed-we-already-did
            let my_action = valid
                .iter()
                .find(|va| va.player_id == seat && !matches!(va.kind, ValidActionKind::Bet { .. }));

            let va = match my_action {
                Some(va) => va.clone(),
                None => break,
            };

            match &va.kind {
                ValidActionKind::CommitSeed => {
                    let commitment = crypto::blake2b(&self.seed)?;
                    self.state.apply(&Action::CommitSeed {
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
                ValidActionKind::ShuffleDeck => {
                    let mut encrypted = self.keys.encrypt_deck(&self.state.game.deck)?;
                    let mut rng = PlayerRng::new(&self.seed, b"shuffle_permutation")?;
                    encrypted.shuffle(rng.as_rng());

                    let deck_bytes: Vec<Bytes> = encrypted
                        .iter()
                        .map(|p| Bytes::copy_from_slice(&p.0))
                        .collect();

                    self.state.apply(&Action::ShuffleDeck {
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
                ValidActionKind::LockDeck => {
                    let deck_hash =
                        crypto::blake2b(&serde_json::to_vec(&self.state.game.deck).unwrap())?;
                    let mut context = b"lock:".to_vec();
                    context.extend_from_slice(&deck_hash);
                    let mut rng = PlayerRng::new(&self.seed, &context)?;
                    self.keys.generate_lock_keys(52, &mut rng)?;
                    let locked = self.keys.lock_deck(&self.state.game.deck)?;

                    let deck_bytes: Vec<Bytes> = locked
                        .iter()
                        .map(|p| Bytes::copy_from_slice(&p.0))
                        .collect();

                    self.state.apply(&Action::LockDeck {
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
                ValidActionKind::RevealLockKey { deck_position } => {
                    let pos = *deck_position;
                    let scalar = self.keys.lock_decrypt[pos].clone();

                    self.state.apply(&Action::RevealLockKey {
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
                ValidActionKind::RevealHand => {
                    let positions = &self.state.hole_card_positions[seat];
                    let scalars: Vec<(usize, Scalar)> = positions
                        .iter()
                        .map(|pos| (*pos, self.keys.lock_decrypt[*pos].clone()))
                        .collect();

                    let reveals: Vec<PositionScalar> = scalars
                        .iter()
                        .map(|(pos, s)| PositionScalar {
                            deck_position: *pos as i64,
                            scalar: Bytes::copy_from_slice(&s.0),
                            extra_data: None,
                        })
                        .collect();

                    self.state.apply(&Action::RevealHand {
                        player_id: seat,
                        scalars,
                    })?;

                    emitted.push(self.encode_action_union(&ActionAction::RevealHand(Box::new(
                        RevealHand {
                            reveals,
                            extra_data: None,
                        },
                    )))?);
                    self.seq += 1;
                }
                ValidActionKind::VerifySeed => {
                    self.state.apply(&Action::VerifySeed {
                        player_id: seat,
                        seed: self.seed.clone(),
                    })?;

                    emitted.push(self.encode_action_union(&ActionAction::VerifySeed(Box::new(
                        VerifySeed {
                            seed: Bytes::copy_from_slice(&self.seed),
                            extra_data: None,
                        },
                    )))?);
                    self.seq += 1;
                }
                ValidActionKind::Bet { .. } => {
                    // Don't auto-respond to bets
                    break;
                }
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
    fn lex_action_to_internal(
        &self,
        action: &ActionAction<'_>,
        _seq: i64,
    ) -> crate::Result<Action> {
        // Figure out which player this is from based on valid_actions
        let valid = self.state.valid_actions();

        match action {
            ActionAction::CommitSeed(cs) => {
                let player_id =
                    find_player_for_action(&valid, |k| matches!(k, ValidActionKind::CommitSeed))?;
                let mut commitment = [0u8; crypto::HASH_BYTES];
                commitment.copy_from_slice(&cs.commitment);
                Ok(Action::CommitSeed {
                    player_id,
                    commitment,
                })
            }
            ActionAction::ShuffleDeck(sd) => {
                let player_id =
                    find_player_for_action(&valid, |k| matches!(k, ValidActionKind::ShuffleDeck))?;
                let deck: Vec<Point> = sd
                    .deck
                    .iter()
                    .map(|b| {
                        let mut arr = [0u8; crypto::POINT_BYTES];
                        arr.copy_from_slice(b);
                        Point(arr)
                    })
                    .collect();
                Ok(Action::ShuffleDeck { player_id, deck })
            }
            ActionAction::LockDeck(ld) => {
                let player_id =
                    find_player_for_action(&valid, |k| matches!(k, ValidActionKind::LockDeck))?;
                let deck: Vec<Point> = ld
                    .deck
                    .iter()
                    .map(|b| {
                        let mut arr = [0u8; crypto::POINT_BYTES];
                        arr.copy_from_slice(b);
                        Point(arr)
                    })
                    .collect();
                Ok(Action::LockDeck { player_id, deck })
            }
            ActionAction::RevealLockKey(rlk) => {
                let pos = rlk.deck_position as usize;
                let player_id = find_player_for_action(
                    &valid,
                    |k| matches!(k, ValidActionKind::RevealLockKey { deck_position } if *deck_position == pos),
                )?;
                let mut scalar_arr = [0u8; crypto::SCALAR_BYTES];
                scalar_arr.copy_from_slice(&rlk.scalar);
                Ok(Action::RevealLockKey {
                    player_id,
                    deck_position: pos,
                    scalar: Scalar(scalar_arr),
                })
            }
            ActionAction::Bet(bet) => {
                let player_id =
                    find_player_for_action(&valid, |k| matches!(k, ValidActionKind::Bet { .. }))?;
                let action = lex_bet_to_internal(bet);
                Ok(Action::Bet { player_id, action })
            }
            ActionAction::RevealHand(rh) => {
                // Match by deck positions — each player has unique hole card positions
                let reveal_positions: Vec<usize> = rh
                    .reveals
                    .iter()
                    .map(|ps| ps.deck_position as usize)
                    .collect();
                let player_id = self
                    .state
                    .hole_card_positions
                    .iter()
                    .enumerate()
                    .find(|(i, positions)| {
                        *positions == &reveal_positions && !self.state.showdown_revealed[*i]
                    })
                    .map(|(i, _)| i)
                    .ok_or_else(|| {
                        crate::Error::InvalidAction(
                            "reveal positions don't match any player".into(),
                        )
                    })?;
                let scalars: Vec<(usize, Scalar)> = rh
                    .reveals
                    .iter()
                    .map(|ps| {
                        let mut s = [0u8; crypto::SCALAR_BYTES];
                        s.copy_from_slice(&ps.scalar);
                        (ps.deck_position as usize, Scalar(s))
                    })
                    .collect();
                Ok(Action::RevealHand { player_id, scalars })
            }
            ActionAction::VerifySeed(vs) => {
                // Match this seed to a player by checking against commitments
                let seed_bytes = vs.seed.to_vec();
                let hash = crypto::blake2b(&seed_bytes)?;
                let player_id = self
                    .state
                    .seed_commitments
                    .iter()
                    .enumerate()
                    .find(|(i, c)| {
                        c.map_or(false, |commitment| commitment == hash)
                            && !self.state.seeds_verified[*i]
                    })
                    .map(|(i, _)| i)
                    .ok_or_else(|| {
                        crate::Error::InvalidAction(
                            "seed doesn't match any unverified commitment".into(),
                        )
                    })?;
                Ok(Action::VerifySeed {
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
    // First decode as generic Value to extract $type
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
        "re.cardco.poker.defs#commitSeed" => {
            let cs: CommitSeed = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode commitSeed: {}", e)))?;
            Ok(ActionAction::CommitSeed(Box::new(cs)))
        }
        "re.cardco.poker.defs#shuffleDeck" => {
            let sd: ShuffleDeck = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode shuffleDeck: {}", e)))?;
            Ok(ActionAction::ShuffleDeck(Box::new(sd)))
        }
        "re.cardco.poker.defs#lockDeck" => {
            let ld: LockDeck = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode lockDeck: {}", e)))?;
            Ok(ActionAction::LockDeck(Box::new(ld)))
        }
        "re.cardco.poker.defs#revealLockKey" => {
            let rlk: RevealLockKey = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode revealLockKey: {}", e)))?;
            Ok(ActionAction::RevealLockKey(Box::new(rlk)))
        }
        "re.cardco.poker.defs#bet" => {
            let bet: Bet = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode bet: {}", e)))?;
            Ok(ActionAction::Bet(Box::new(bet)))
        }
        "re.cardco.poker.defs#revealHand" => {
            let rh: RevealHand = dasl::drisl::from_slice(cbor)
                .map_err(|e| crate::Error::Protocol(format!("decode revealHand: {}", e)))?;
            Ok(ActionAction::RevealHand(Box::new(rh)))
        }
        "re.cardco.poker.defs#verifySeed" => {
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

/// Find which player should be performing an action of this type.
fn find_player_for_action(
    valid: &[protocol::ValidAction],
    predicate: impl Fn(&ValidActionKind) -> bool,
) -> crate::Result<usize> {
    valid
        .iter()
        .find(|va| predicate(&va.kind))
        .map(|va| va.player_id)
        .ok_or_else(|| crate::Error::InvalidAction("no valid action of this type".into()))
}

fn bet_action_to_lex(action: &BetAction) -> BetAction2 {
    match action {
        BetAction::Fold => BetAction2::Fold,
        BetAction::Check => BetAction2::Check,
        BetAction::Call => BetAction2::Call,
        BetAction::AllIn => BetAction2::AllIn,
        BetAction::Raise(_) => BetAction2::Other("raise".into()),
    }
}

fn lex_bet_to_internal(bet: &Bet<'_>) -> BetAction {
    match &bet.action {
        BetAction2::Fold => BetAction::Fold,
        BetAction2::Check => BetAction::Check,
        BetAction2::Call => BetAction::Call,
        BetAction2::AllIn => BetAction::AllIn,
        BetAction2::Other(s) if s.as_ref() == "raise" => {
            BetAction::Raise(bet.amount.unwrap_or(0) as u64)
        }
        _ => BetAction::Fold, // Unknown action treated as fold
    }
}

// Alias to avoid confusion with our internal BetAction
use crate::lexicon::re_cardco::poker::BetAction as BetAction2;
