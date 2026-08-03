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
use crate::lexicon::re_cardco::poker::action::ActionAction;
use crate::lexicon::re_cardco::poker::table::Table as LexTable;
use crate::lexicon::re_cardco::poker::*;
use crate::protocol::{Action, Phase, ProtocolState, ValidActionKind};

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
    /// The long-lived secret seed supplied at construction. Per-hand seeds are
    /// derived from it so each hand uses fresh randomness while a single hand's
    /// seed can still be revealed without compromising the others.
    master_seed: Vec<u8>,
    keys: PlayerKeys,
    state: ProtocolState,
    seat: Option<usize>,
    /// The table roster, in seat order. Used to attribute received actions to
    /// the seat of the repo they came from — attribution must come from the
    /// author, never be inferred from whose turn the state machine thinks it
    /// is, or concurrent replay ends up applying actions to the wrong player.
    player_dids: Vec<String>,
    seq: i64,
    table_tid: Option<String>,
    /// Which hand we're playing — must track protocol.hand_index for key derivation.
    hand_index: u64,
    /// Chained (settlement) mode. When true the agent stops eagerly emitting and
    /// applying its own actions. Instead the caller drives one bounded, globally
    /// ordered action chain: it asks `next_action()` who acts next in the
    /// canonical `valid_actions()[0]` order, calls `produce_next()` /
    /// `produce_bet()` to build (but NOT apply) this seat's action, publishes it
    /// with an explicit `prev`/global `seq`, and feeds every action — its own and
    /// peers' — back through `receive_action` in chain order. This is what makes
    /// a paid hand settlement-valid: a single contiguous chain that ends with
    /// every seat's `VerifySeed`, exactly like the offline transcript generator.
    chained: bool,
}

impl PlayerAgent {
    /// Create a new agent with the player's DID and secret seed.
    pub fn new(did: &str, seed: &[u8]) -> crate::Result<Self> {
        crypto::init()?;
        let master_seed = seed.to_vec();
        let phs = per_hand_seed(&master_seed, 0)?;
        let mut rng = PlayerRng::new(&phs, b"shuffle")?;
        let keys = PlayerKeys::generate(&mut rng)?;
        Ok(Self {
            did: did.to_string(),
            master_seed,
            keys,
            state: ProtocolState::new(),
            seat: None,
            player_dids: Vec::new(),
            seq: 0,
            table_tid: None,
            hand_index: 0,
            chained: false,
        })
    }

    /// Turn chained (settlement) driving on or off. In chained mode the agent
    /// never auto-emits: the caller drives one globally ordered chain via
    /// `next_action()` + `produce_next()`/`produce_bet()` and feeds every action
    /// back through `receive_action`. Must be set before the table is received.
    pub fn set_chained(&mut self, on: bool) {
        self.chained = on;
    }

    /// Whether chained (settlement) driving is enabled.
    pub fn is_chained(&self) -> bool {
        self.chained
    }

    /// Seed for the current hand, derived from the master seed and hand index.
    fn per_hand_seed(&self) -> crate::Result<Vec<u8>> {
        per_hand_seed(&self.master_seed, self.hand_index)
    }

    /// Regenerate this hand's shuffle/lock keys from the current per-hand seed.
    fn rederive_keys(&mut self) -> crate::Result<()> {
        let phs = self.per_hand_seed()?;
        let mut rng = PlayerRng::new(&phs, b"shuffle")?;
        self.keys = PlayerKeys::generate(&mut rng)?;
        Ok(())
    }

    /// Advance to the next hand once the current one is Complete. Rotates the
    /// button, rederives fresh keys, and auto-emits this player's new CommitSeed.
    pub fn next_hand(&mut self) -> crate::Result<AgentOutput> {
        if self.state.game_over() {
            return Ok(AgentOutput::Waiting);
        }
        self.state.start_next_hand();
        self.hand_index = self.state.hand_index;
        self.rederive_keys()?;
        self.auto_respond()
    }

    /// JSON of the most recently completed hand's result, if any.
    pub fn last_hand_result_json(&self) -> Option<String> {
        self.state
            .last_hand_result
            .as_ref()
            .and_then(|r| serde_json::to_string(r).ok())
    }

    /// Whether the whole game is over (at most one player with chips).
    pub fn game_over(&self) -> bool {
        self.state.game_over()
    }

    /// Feed a DAG-CBOR encoded table record. This starts the game.
    pub fn receive_table(&mut self, cbor: &[u8]) -> crate::Result<AgentOutput> {
        let table: LexTable = dasl::drisl::from_slice(cbor)
            .map_err(|e| crate::Error::Protocol(format!("invalid table CBOR: {}", e)))?;

        // Find our seat
        // A DID that isn't in the roster becomes a SPECTATOR: seat stays None,
        // the state machine tracks every action like a player's would (so the
        // whole game can be replayed from public PDS records), but the agent
        // never emits actions and can't bet. Hole cards stay encrypted to a
        // spectator until players reveal them at showdown — the protocol's
        // privacy doesn't depend on who's watching.
        self.seat = table
            .players
            .iter()
            .position(|did| did.as_str() == self.did);

        let players: Vec<String> = table
            .players
            .iter()
            .map(|d| d.as_str().to_string())
            .collect();
        self.player_dids = players.clone();

        // Apply table to protocol state
        self.state.apply(&Action::Table {
            players,
            starting_chips: table.starting_chips as u64,
            small_blind: table.small_blind as u64,
        })?;

        // Now auto-respond with any actions we can take
        self.auto_respond()
    }

    /// Feed a DAG-CBOR encoded action payload authored by `author_did` (the
    /// DID of the repo the record was fetched from). The payload is a map with
    /// a `$type` field for dispatch.
    ///
    /// The author identity is load-bearing for consensus: the action is
    /// attributed to the author's seat and the state machine validates it for
    /// that seat. Attribution must never be inferred from whose turn it is —
    /// during a concurrent multi-repo replay, actions arrive in arbitrary
    /// interleavings, and turn-order guessing applies them to the wrong player.
    pub fn receive_action(&mut self, cbor: &[u8], author_did: &str) -> crate::Result<AgentOutput> {
        let author_seat = self
            .player_dids
            .iter()
            .position(|d| d == author_did)
            .ok_or_else(|| {
                crate::Error::InvalidAction(format!("author {} is not at the table", author_did))
            })?;
        let action = decode_action_cbor(cbor)?;
        let internal_action = self.lex_action_to_internal(&action, author_seat)?;
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

    // --- Chained (settlement) driving ---------------------------------------
    //
    // These mirror the offline transcript generator: the canonical global order
    // is always `state.valid_actions()[0]`. Every agent that has applied the
    // same chain prefix computes the same next action, so N independent repos
    // build one contiguous chain that ends with every seat's VerifySeed.

    /// JSON describing the next action in the canonical global order, or
    /// `"null"` when nothing is pending (all seeds verified → chain complete):
    /// `{"seat":1,"kind":"shuffleDeck","mine":true}`. Bet entries also carry
    /// `deckPosition` (for revealLockKey) and `options` (for bet).
    pub fn next_action_json(&self) -> String {
        let valid = self.state.valid_actions();
        let Some(va) = valid.first() else {
            return "null".to_string();
        };
        let (kind, pos) = valid_kind_name(&va.kind);
        let mut value = serde_json::json!({
            "seat": va.player_id,
            "kind": kind,
            "mine": Some(va.player_id) == self.seat,
        });
        if let Some(position) = pos {
            value["deckPosition"] = position.into();
        }
        if let ValidActionKind::Bet { options } = &va.kind {
            value["options"] =
                serde_json::to_value(options).unwrap_or(serde_json::Value::Array(Vec::new()));
        }
        serde_json::to_string(&value).unwrap_or_else(|_| "null".into())
    }

    /// Build (but do NOT apply) this seat's next canonical action when the
    /// global order is on us and the action is non-interactive (commit, shuffle,
    /// lock, reveal, or seed verification). Returns an empty vector when it is
    /// not our turn or the next action needs a betting decision (use
    /// `produce_bet`). The caller publishes the returned CBOR with an explicit
    /// `prev`/`seq`, then feeds it back through `receive_action` to apply it.
    pub fn produce_next(&mut self) -> crate::Result<Vec<u8>> {
        let seat = match self.seat {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };
        let valid = self.state.valid_actions();
        let Some(va) = valid.first() else {
            return Ok(Vec::new());
        };
        if va.player_id != seat {
            return Ok(Vec::new());
        }
        let kind = va.kind.clone();
        if matches!(kind, ValidActionKind::Bet { .. }) {
            return Ok(Vec::new());
        }
        let (_action, cbor) = self.build_action(seat, &kind)?;
        Ok(cbor)
    }

    /// Build (but do NOT apply) this seat's betting action for the current
    /// canonical turn. Mirrors `bet()` without touching state; the caller
    /// publishes the CBOR and feeds it back through `receive_action`.
    pub fn produce_bet(&mut self, action: BetAction) -> crate::Result<Vec<u8>> {
        let amount = match &action {
            BetAction::Raise(amt) => Some(*amt as i64),
            _ => None,
        };
        let lex_bet = Bet {
            action: bet_action_to_lex(&action),
            amount,
            extra_data: None,
        };
        self.encode_action_union(&ActionAction::Bet(Box::new(lex_bet)))
    }

    /// Construct the internal action and its DAG-CBOR payload for `seat`'s
    /// `kind` without applying it to protocol state. Shared by the eager
    /// (auto-emit) path and the chained (produce-without-apply) path so both
    /// produce byte-identical records. Lock-key generation mutates `self.keys`
    /// as a side effect — this is deterministic from the per-hand seed and the
    /// current deck, and idempotent across retries.
    fn build_action(
        &mut self,
        seat: usize,
        kind: &ValidActionKind,
    ) -> crate::Result<(Action, Vec<u8>)> {
        match kind {
            ValidActionKind::CommitSeed => {
                let commitment = crypto::blake2b(&self.per_hand_seed()?)?;
                let action = Action::CommitSeed {
                    player_id: seat,
                    commitment,
                };
                let cbor = self.encode_action_union(&ActionAction::CommitSeed(Box::new(
                    CommitSeed {
                        commitment: Bytes::copy_from_slice(&commitment),
                        extra_data: None,
                    },
                )))?;
                Ok((action, cbor))
            }
            ValidActionKind::ShuffleDeck => {
                let mut encrypted = self.keys.encrypt_deck(&self.state.game.deck)?;
                let phs = self.per_hand_seed()?;
                let mut rng = PlayerRng::new(&phs, b"shuffle_permutation")?;
                encrypted.shuffle(rng.as_rng());
                let deck_bytes: Vec<Bytes> = encrypted
                    .iter()
                    .map(|p| Bytes::copy_from_slice(&p.0))
                    .collect();
                let action = Action::ShuffleDeck {
                    player_id: seat,
                    deck: encrypted,
                };
                let cbor = self.encode_action_union(&ActionAction::ShuffleDeck(Box::new(
                    ShuffleDeck {
                        deck: deck_bytes,
                        extra_data: None,
                    },
                )))?;
                Ok((action, cbor))
            }
            ValidActionKind::LockDeck => {
                let deck_hash =
                    crypto::blake2b(&serde_json::to_vec(&self.state.game.deck).unwrap())?;
                let mut context = b"lock:".to_vec();
                context.extend_from_slice(&deck_hash);
                let phs = self.per_hand_seed()?;
                let mut rng = PlayerRng::new(&phs, &context)?;
                self.keys.generate_lock_keys(52, &mut rng)?;
                let locked = self.keys.lock_deck(&self.state.game.deck)?;
                let deck_bytes: Vec<Bytes> = locked
                    .iter()
                    .map(|p| Bytes::copy_from_slice(&p.0))
                    .collect();
                let action = Action::LockDeck {
                    player_id: seat,
                    deck: locked,
                };
                let cbor = self.encode_action_union(&ActionAction::LockDeck(Box::new(LockDeck {
                    deck: deck_bytes,
                    extra_data: None,
                })))?;
                Ok((action, cbor))
            }
            ValidActionKind::RevealLockKey { deck_position } => {
                let pos = *deck_position;
                let scalar = self.keys.lock_decrypt[pos].clone();
                let action = Action::RevealLockKey {
                    player_id: seat,
                    deck_position: pos,
                    scalar: scalar.clone(),
                };
                let cbor = self.encode_action_union(&ActionAction::RevealLockKey(Box::new(
                    RevealLockKey {
                        deck_position: pos as i64,
                        scalar: Bytes::copy_from_slice(&scalar.0),
                        extra_data: None,
                    },
                )))?;
                Ok((action, cbor))
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
                let action = Action::RevealHand {
                    player_id: seat,
                    scalars,
                };
                let cbor = self.encode_action_union(&ActionAction::RevealHand(Box::new(
                    RevealHand {
                        reveals,
                        extra_data: None,
                    },
                )))?;
                Ok((action, cbor))
            }
            ValidActionKind::VerifySeed => {
                let seed = self.per_hand_seed()?;
                let action = Action::VerifySeed {
                    player_id: seat,
                    seed: seed.clone(),
                };
                let cbor = self.encode_action_union(&ActionAction::VerifySeed(Box::new(
                    VerifySeed {
                        seed: Bytes::copy_from_slice(&seed),
                        extra_data: None,
                    },
                )))?;
                Ok((action, cbor))
            }
            ValidActionKind::Bet { .. } => Err(crate::Error::Protocol(
                "build_action cannot construct a bet; use produce_bet".into(),
            )),
        }
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

    /// JSON summary of which protocol step(s) the game is blocked on and which
    /// seats owe them: `[{"kind":"shuffleDeck","seats":[1]}, ...]`. Kinds use
    /// the lexicon's camelCase names so they match the action log. This is the
    /// debugging view for stalls — every window can show "waiting on
    /// shuffleDeck from X" and a disagreement between windows localizes the
    /// missed or rejected action.
    pub fn waiting_on_json(&self) -> String {
        use std::collections::BTreeMap;
        // Group seats by (kind, deck position) — e.g. all the players who
        // still owe a RevealLockKey for the card being dealt.
        let mut groups: BTreeMap<(&'static str, Option<usize>), Vec<usize>> = BTreeMap::new();
        for va in self.state.valid_actions() {
            let (kind, pos) = match &va.kind {
                ValidActionKind::CommitSeed => ("commitSeed", None),
                ValidActionKind::ShuffleDeck => ("shuffleDeck", None),
                ValidActionKind::LockDeck => ("lockDeck", None),
                ValidActionKind::RevealLockKey { deck_position } => {
                    ("revealLockKey", Some(*deck_position))
                }
                ValidActionKind::Bet { .. } => ("bet", None),
                ValidActionKind::RevealHand => ("revealHand", None),
                ValidActionKind::VerifySeed => ("verifySeed", None),
            };
            groups.entry((kind, pos)).or_default().push(va.player_id);
        }
        let entries: Vec<serde_json::Value> = groups
            .into_iter()
            .map(|((kind, pos), seats)| {
                let mut v = serde_json::json!({ "kind": kind, "seats": seats });
                if let Some(p) = pos {
                    v["deckPosition"] = p.into();
                }
                v
            })
            .collect();
        serde_json::to_string(&entries).unwrap_or_else(|_| "[]".into())
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
                    "eliminated": p.eliminated,
                })
            })
            .collect();
        serde_json::to_string(&serde_json::json!({
            "pot": state.pot,
            "currentBet": state.current_bet,
            "actionOn": state.action_on,
            "button": state.button,
            "handIndex": self.state.hand_index,
            "gameOver": self.state.game_over(),
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
        // Chained mode drives every action explicitly through the caller so a
        // single globally ordered chain can be built. Never auto-emit here.
        if self.chained {
            return Ok(Vec::new());
        }
        let mut emitted = Vec::new();
        loop {
            let valid = self.state.valid_actions();
            let seat = match self.seat {
                Some(s) => s,
                None => break,
            };

            // Find a non-interactive action for us (not a bet, and not seed
            // verification — seeds aren't auto-revealed in a multi-hand game).
            let my_action = valid.iter().find(|va| {
                va.player_id == seat
                    && !matches!(
                        va.kind,
                        ValidActionKind::Bet { .. } | ValidActionKind::VerifySeed
                    )
            });

            let va = match my_action {
                Some(va) => va.clone(),
                None => break,
            };

            // Build the action and its payload once (shared with the chained
            // produce-without-apply path), then apply it and emit the CBOR.
            let (action, cbor) = self.build_action(seat, &va.kind)?;
            self.state.apply(&action)?;
            emitted.push(cbor);
            self.seq += 1;
        }
        Ok(emitted)
    }

    /// Encode an ActionAction union variant as DAG-CBOR.
    /// The $type tag is included for dispatch on the receiving end.
    fn encode_action_union(&self, action: &ActionAction<'_>) -> crate::Result<Vec<u8>> {
        dasl::drisl::to_vec(action)
            .map_err(|e| crate::Error::Protocol(format!("CBOR encode failed: {}", e)))
    }

    /// Convert a lexicon action to an internal protocol action, attributed to
    /// the author's seat. The state machine then validates the action for that
    /// specific seat (turn order, duplicate commits/reveals, phase).
    fn lex_action_to_internal(
        &self,
        action: &ActionAction<'_>,
        author_seat: usize,
    ) -> crate::Result<Action> {
        match action {
            ActionAction::CommitSeed(cs) => {
                let player_id = author_seat;
                let mut commitment = [0u8; crypto::HASH_BYTES];
                commitment.copy_from_slice(&cs.commitment);
                Ok(Action::CommitSeed {
                    player_id,
                    commitment,
                })
            }
            ActionAction::ShuffleDeck(sd) => {
                let player_id = author_seat;
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
                let player_id = author_seat;
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
                let player_id = author_seat;
                let mut scalar_arr = [0u8; crypto::SCALAR_BYTES];
                scalar_arr.copy_from_slice(&rlk.scalar);
                Ok(Action::RevealLockKey {
                    player_id,
                    deck_position: pos,
                    scalar: Scalar(scalar_arr),
                })
            }
            ActionAction::Bet(bet) => {
                let action = lex_bet_to_internal(bet);
                Ok(Action::Bet {
                    player_id: author_seat,
                    action,
                })
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
                if player_id != author_seat {
                    return Err(crate::Error::InvalidAction(format!(
                        "revealHand positions belong to seat {} but the record was authored by seat {}",
                        player_id, author_seat
                    )));
                }
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
                if player_id != author_seat {
                    return Err(crate::Error::InvalidAction(format!(
                        "seed matches seat {}'s commitment but the record was authored by seat {}",
                        player_id, author_seat
                    )));
                }
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

/// Derive a per-hand secret seed from the master seed and hand index. Using a
/// one-way hash means a revealed per-hand seed can't be used to recover the
/// master seed or any other hand's seed.
fn per_hand_seed(master_seed: &[u8], hand_index: u64) -> crate::Result<Vec<u8>> {
    let mut data = Vec::with_capacity(master_seed.len() + 8);
    data.extend_from_slice(master_seed);
    data.extend_from_slice(&hand_index.to_le_bytes());
    Ok(crypto::blake2b(&data)?.to_vec())
}

/// Map a valid-action kind to its lexicon camelCase name and optional deck
/// position, matching `waiting_on_json` and the transcript record `$type`s.
fn valid_kind_name(kind: &ValidActionKind) -> (&'static str, Option<usize>) {
    match kind {
        ValidActionKind::CommitSeed => ("commitSeed", None),
        ValidActionKind::ShuffleDeck => ("shuffleDeck", None),
        ValidActionKind::LockDeck => ("lockDeck", None),
        ValidActionKind::RevealLockKey { deck_position } => ("revealLockKey", Some(*deck_position)),
        ValidActionKind::Bet { .. } => ("bet", None),
        ValidActionKind::RevealHand => ("revealHand", None),
        ValidActionKind::VerifySeed => ("verifySeed", None),
    }
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
