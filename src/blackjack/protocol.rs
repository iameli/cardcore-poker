//! Blackjack protocol state machine on the shared mental-card engine.
//!
//! Round flow (ENHC, rotating banker):
//! 1. Commit seeds → shuffle → lock (shared `CryptoRound` mechanics)
//! 2. Wagering in seat order, left of the banker
//! 3. Initial deal, all face-up: one card to each bettor, the banker's
//!    upcard, then each bettor's second card
//! 4. Insurance offers (only when the upcard is an ace)
//! 5. Player turns in seat order (split hand 0 before hand 1); hit, double,
//!    and split schedule one more face-up deal each
//! 6. Banker draws face-up to 17, standing on soft 17 (the banker-draw stage
//!    is `Dealing { target: Banker }`)
//! 7. Settlement → `Complete` → post-game seed verification
//!
//! Every deal is a public N-of-N reveal (`exclude = None`) — blackjack has no
//! hidden cards, so revealed points map straight to plain `Card`s.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::blackjack::game::{BlackjackGame, Decision, RoundResult, TurnNeed};
use crate::card::Card;
use crate::crypto::{self, Point, Scalar};
use crate::engine::{CryptoRound, DECK_SIZE, PlayerId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BjPhase {
    /// No table established yet.
    Init,
    /// Players commit hashes of their secret seeds.
    CommitSeeds,
    /// Players take turns encrypting and shuffling the deck.
    Shuffle { next_player: PlayerId },
    /// Players take turns swapping shuffle keys for per-position lock keys.
    Lock { next_player: PlayerId },
    /// Bettors post wagers in seat order.
    Wagering { action_on: PlayerId },
    /// Players reveal lock scalars to deal the card at `deck_position`
    /// face-up to `target` (the banker-draw stage is `target: Banker`).
    Dealing {
        target: DealTarget,
        deck_position: usize,
    },
    /// Bettors take or decline insurance (banker upcard is an ace).
    Insurance { action_on: PlayerId },
    /// A bettor decides hit/stand/double/split/surrender for one hand.
    PlayerTurn { action_on: PlayerId, hand: usize },
    /// Round settled; seeds may be revealed for verification.
    Complete,
}

/// Where the card being dealt goes. All deals are public.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DealTarget {
    PlayerHand { player: PlayerId, hand: usize },
    Banker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BjAction {
    /// Establish the table: players, chips, minimum bet. First action.
    Table {
        players: Vec<String>,
        starting_chips: u64,
        min_bet: u64,
    },
    CommitSeed {
        player_id: PlayerId,
        #[serde(with = "crypto::serde_base64")]
        commitment: [u8; crypto::HASH_BYTES],
    },
    ShuffleDeck {
        player_id: PlayerId,
        deck: Vec<Point>,
    },
    LockDeck {
        player_id: PlayerId,
        deck: Vec<Point>,
    },
    /// Reveal a per-position lock scalar for dealing.
    RevealLockKey {
        player_id: PlayerId,
        deck_position: usize,
        scalar: Scalar,
    },
    Wager {
        player_id: PlayerId,
        amount: u64,
    },
    Insurance {
        player_id: PlayerId,
        take: bool,
    },
    Decision {
        player_id: PlayerId,
        decision: Decision,
    },
    /// Post-game: reveal secret seed for full verification.
    VerifySeed {
        player_id: PlayerId,
        #[serde(with = "crypto::serde_base64_vec")]
        seed: Vec<u8>,
    },
}

impl BjAction {
    /// Returns the player who submitted this action, if applicable.
    pub fn player_id(&self) -> Option<PlayerId> {
        match self {
            BjAction::Table { .. } => None,
            BjAction::CommitSeed { player_id, .. }
            | BjAction::ShuffleDeck { player_id, .. }
            | BjAction::LockDeck { player_id, .. }
            | BjAction::RevealLockKey { player_id, .. }
            | BjAction::Wager { player_id, .. }
            | BjAction::Insurance { player_id, .. }
            | BjAction::Decision { player_id, .. }
            | BjAction::VerifySeed { player_id, .. } => Some(*player_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BjValidAction {
    pub player_id: PlayerId,
    pub kind: BjValidActionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BjValidActionKind {
    CommitSeed,
    ShuffleDeck,
    LockDeck,
    RevealLockKey { deck_position: usize },
    Wager { min: u64, max: u64 },
    Insurance,
    Decision { options: Vec<Decision> },
    VerifySeed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BjProtocolState {
    pub phase: BjPhase,
    pub game: BlackjackGame,
    /// Shared mental-card crypto state (deck, commitments, reveals) for the
    /// current round.
    pub crypto: CryptoRound,
    /// Decrypted point → card lookup (every blackjack card is public).
    card_map: HashMap<Point, Card>,
    /// Remaining deal targets; the head is the card currently being dealt.
    deal_queue: Vec<DealTarget>,
    /// The one-time insurance stage has been offered (or skipped).
    insurance_done: bool,
    /// Which round we're on (0-based). Bumped each time a new round starts.
    pub hand_index: u64,
    /// Result of the most recently completed round (for the UI log).
    pub last_round_result: Option<RoundResult>,
}

impl BjProtocolState {
    pub fn new() -> Self {
        Self {
            phase: BjPhase::Init,
            game: BlackjackGame::new(0, 0, 0),
            crypto: CryptoRound::new(0),
            card_map: HashMap::new(),
            deal_queue: Vec::new(),
            insurance_done: false,
            hand_index: 0,
            last_round_result: None,
        }
    }

    pub fn apply(&mut self, action: &BjAction) -> crate::Result<()> {
        match (&self.phase, action) {
            // --- Table ---
            (
                BjPhase::Init,
                BjAction::Table {
                    players,
                    starting_chips,
                    min_bet,
                },
            ) => {
                let n = players.len();
                if n < 2 || n > 6 {
                    return Err(crate::Error::InvalidAction("need 2-6 players".into()));
                }
                if *min_bet < 1 {
                    return Err(crate::Error::InvalidAction("min bet must be >= 1".into()));
                }
                self.game = BlackjackGame::new(n, *starting_chips, *min_bet);
                self.crypto = CryptoRound::new(n);
                self.phase = BjPhase::CommitSeeds;
                Ok(())
            }

            // --- Commit Seeds ---
            (
                BjPhase::CommitSeeds,
                BjAction::CommitSeed {
                    player_id,
                    commitment,
                },
            ) => {
                let all_live_committed =
                    self.crypto
                        .apply_commit_seed(*player_id, *commitment, &self.live_seats())?;
                if all_live_committed {
                    let card_points = crypto::card_points()?;
                    self.card_map = card_points.iter().map(|(c, p)| (p.clone(), *c)).collect();
                    self.crypto.deck = card_points.into_iter().map(|(_, p)| p).collect();
                    self.phase = BjPhase::Shuffle {
                        next_player: self.live_seats()[0],
                    };
                }
                Ok(())
            }

            // --- Shuffle ---
            (BjPhase::Shuffle { next_player }, BjAction::ShuffleDeck { player_id, deck }) => {
                let live = self.live_seats();
                match self
                    .crypto
                    .apply_shuffle(*player_id, *next_player, deck, &live)?
                {
                    Some(next) => {
                        self.phase = BjPhase::Shuffle { next_player: next };
                    }
                    None => {
                        self.phase = BjPhase::Lock {
                            next_player: live[0],
                        };
                    }
                }
                Ok(())
            }

            // --- Lock ---
            (BjPhase::Lock { next_player }, BjAction::LockDeck { player_id, deck }) => {
                let live = self.live_seats();
                match self
                    .crypto
                    .apply_lock(*player_id, *next_player, deck, &live)?
                {
                    Some(next) => {
                        self.phase = BjPhase::Lock { next_player: next };
                    }
                    None => {
                        // Deck fixed — wagering opens, left of the banker.
                        let first = self
                            .game
                            .next_wagerer()
                            .ok_or_else(|| crate::Error::Protocol("no bettors to wager".into()))?;
                        self.phase = BjPhase::Wagering { action_on: first };
                    }
                }
                Ok(())
            }

            // --- Wagering ---
            (BjPhase::Wagering { action_on }, BjAction::Wager { player_id, amount }) => {
                if *player_id != *action_on {
                    return Err(crate::Error::InvalidAction("not your turn to wager".into()));
                }
                self.game.apply_wager(*player_id, *amount)?;
                match self.game.next_wagerer() {
                    Some(next) => {
                        self.phase = BjPhase::Wagering { action_on: next };
                    }
                    None => self.start_initial_deal(),
                }
                Ok(())
            }

            // --- Reveal Lock Key (dealing, always face-up) ---
            (
                BjPhase::Dealing {
                    target,
                    deck_position,
                },
                BjAction::RevealLockKey {
                    player_id,
                    deck_position: action_pos,
                    scalar,
                },
            ) => {
                let live = self.live_seats();
                if let Some(point) = self.crypto.apply_reveal(
                    *player_id,
                    *action_pos,
                    *deck_position,
                    scalar,
                    None,
                    &live,
                )? {
                    let target = *target;
                    let card = *self.card_map.get(&point).ok_or_else(|| {
                        crate::Error::Crypto("revealed point is not a card".into())
                    })?;
                    match target {
                        DealTarget::PlayerHand { player, hand } => {
                            self.game.deal_to_hand(player, hand, card);
                        }
                        DealTarget::Banker => {
                            self.game.deal_to_banker(card);
                        }
                    }
                    self.deal_queue.remove(0);
                    if self.deal_queue.is_empty() {
                        self.advance_round();
                    } else {
                        self.start_queued_deal();
                    }
                }
                Ok(())
            }

            // --- Insurance ---
            (BjPhase::Insurance { action_on }, BjAction::Insurance { player_id, take }) => {
                if *player_id != *action_on {
                    return Err(crate::Error::InvalidAction(
                        "not your turn to answer insurance".into(),
                    ));
                }
                self.game.apply_insurance(*player_id, *take)?;
                self.advance_round();
                Ok(())
            }

            // --- Player decisions ---
            (
                BjPhase::PlayerTurn { action_on, hand },
                BjAction::Decision {
                    player_id,
                    decision,
                },
            ) => {
                if *player_id != *action_on {
                    return Err(crate::Error::InvalidAction("not your turn to act".into()));
                }
                let hand = *hand;
                let needs_card = self.game.apply_decision(*player_id, hand, *decision)?;
                if needs_card {
                    // The exact target (split sends it to hand 0) is tracked
                    // by the rules core.
                    let (player, hand) = self.game.pending_card.expect("pending card after move");
                    self.schedule_deal(DealTarget::PlayerHand { player, hand });
                } else {
                    self.advance_round();
                }
                Ok(())
            }

            // --- Verify Seed (post-game) ---
            (BjPhase::Complete, BjAction::VerifySeed { player_id, seed }) => {
                self.crypto.apply_verify_seed(*player_id, seed)
            }

            _ => Err(crate::Error::InvalidAction(format!(
                "action {:?} not valid in phase {:?}",
                std::mem::discriminant(action),
                self.phase
            ))),
        }
    }

    /// Seats still in the game, in seat order. These are the only players who
    /// participate in the cryptographic protocol (commit/shuffle/lock/reveal).
    fn live_seats(&self) -> Vec<usize> {
        self.game.live_seats()
    }

    pub fn valid_actions(&self) -> Vec<BjValidAction> {
        match &self.phase {
            BjPhase::Init => vec![],
            BjPhase::CommitSeeds => self
                .crypto
                .seed_commitments
                .iter()
                .enumerate()
                .filter(|(pid, c)| c.is_none() && !self.game.players[*pid].eliminated)
                .map(|(pid, _)| BjValidAction {
                    player_id: pid,
                    kind: BjValidActionKind::CommitSeed,
                })
                .collect(),
            BjPhase::Shuffle { next_player } => vec![BjValidAction {
                player_id: *next_player,
                kind: BjValidActionKind::ShuffleDeck,
            }],
            BjPhase::Lock { next_player } => vec![BjValidAction {
                player_id: *next_player,
                kind: BjValidActionKind::LockDeck,
            }],
            BjPhase::Wagering { action_on } => {
                let (min, max) = self.game.wager_bounds(*action_on);
                vec![BjValidAction {
                    player_id: *action_on,
                    kind: BjValidActionKind::Wager { min, max },
                }]
            }
            BjPhase::Dealing { deck_position, .. } => self
                .live_seats()
                .into_iter()
                .filter(|pid| !self.crypto.deal_reveals.contains_key(pid))
                .map(|pid| BjValidAction {
                    player_id: pid,
                    kind: BjValidActionKind::RevealLockKey {
                        deck_position: *deck_position,
                    },
                })
                .collect(),
            BjPhase::Insurance { action_on } => vec![BjValidAction {
                player_id: *action_on,
                kind: BjValidActionKind::Insurance,
            }],
            BjPhase::PlayerTurn { action_on, hand } => vec![BjValidAction {
                player_id: *action_on,
                kind: BjValidActionKind::Decision {
                    options: self.game.legal_decisions(*action_on, *hand),
                },
            }],
            BjPhase::Complete => self
                .crypto
                .seeds_verified
                .iter()
                .enumerate()
                .filter(|(_, v)| !**v)
                .map(|(pid, _)| BjValidAction {
                    player_id: pid,
                    kind: BjValidActionKind::VerifySeed,
                })
                .collect(),
        }
    }

    /// Queue the initial deal: one card to each bettor in seat order, the
    /// banker's upcard, then each bettor's second card (ENHC — the banker
    /// gets no second card until every player has acted).
    fn start_initial_deal(&mut self) {
        let bettors = self.game.bettor_seats();
        self.deal_queue.clear();
        for &pid in &bettors {
            self.deal_queue.push(DealTarget::PlayerHand {
                player: pid,
                hand: 0,
            });
        }
        self.deal_queue.push(DealTarget::Banker);
        for &pid in &bettors {
            self.deal_queue.push(DealTarget::PlayerHand {
                player: pid,
                hand: 0,
            });
        }
        self.crypto.next_deck_position = 0;
        self.start_queued_deal();
    }

    /// Begin revealing the card for the deal at the head of the queue.
    fn start_queued_deal(&mut self) {
        let target = self.deal_queue[0];
        let pos = self.crypto.begin_deal();
        self.phase = BjPhase::Dealing {
            target,
            deck_position: pos,
        };
    }

    /// Schedule a single mid-round deal, guarding against deck exhaustion:
    /// if no card is left, the affected hand auto-stands (or the banker
    /// stands on their current total) and the round moves on.
    fn schedule_deal(&mut self, target: DealTarget) {
        if self.crypto.next_deck_position >= DECK_SIZE {
            match target {
                DealTarget::PlayerHand { player, hand } => {
                    self.game.force_stand(player, hand);
                    self.advance_round();
                }
                DealTarget::Banker => self.finish_round(),
            }
            return;
        }
        self.deal_queue.push(target);
        self.start_queued_deal();
    }

    /// Decide what the round needs next. Called whenever the deal queue
    /// drains or a non-dealing action resolves.
    fn advance_round(&mut self) {
        // Insurance offers come once, right after the initial deal.
        if !self.insurance_done {
            if self.game.insurance_offered() {
                if let Some(pid) = self.game.next_insurer() {
                    self.phase = BjPhase::Insurance { action_on: pid };
                    return;
                }
            }
            self.insurance_done = true;
        }
        match self.game.next_turn_need() {
            Some(TurnNeed::Card { player, hand }) => {
                self.schedule_deal(DealTarget::PlayerHand { player, hand });
            }
            Some(TurnNeed::Decision { player, hand }) => {
                self.phase = BjPhase::PlayerTurn {
                    action_on: player,
                    hand,
                };
            }
            None => {
                // Every bettor hand is resolved. The banker always completes
                // their hand to >= 17 (S17) — it also resolves insurance.
                if self.game.banker_must_draw() && self.crypto.next_deck_position < DECK_SIZE {
                    self.schedule_deal(DealTarget::Banker);
                } else {
                    self.finish_round();
                }
            }
        }
    }

    /// Settle the round and move to `Complete`.
    fn finish_round(&mut self) {
        self.last_round_result = Some(self.game.settle(self.hand_index));
        self.phase = BjPhase::Complete;
    }

    /// The game is over once at most one player has chips left.
    pub fn game_over(&self) -> bool {
        self.game.game_over()
    }

    /// Begin the next round: bust broke players, rotate the banker to the
    /// next live seat, clear per-round state, and return to seed commitment.
    /// Caller must ensure the game isn't already over.
    pub fn start_next_round(&mut self) {
        self.game.start_next_round();
        self.crypto.reset_for_next_hand();
        // Each round commits fresh seeds, so verification starts over too.
        self.crypto.seeds_verified = vec![false; self.game.num_players()];
        self.card_map.clear();
        self.deal_queue.clear();
        self.insurance_done = false;
        self.last_round_result = None;
        self.hand_index += 1;
        self.phase = BjPhase::CommitSeeds;
    }
}

impl Default for BjProtocolState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    fn c(rank: Rank) -> Card {
        Card::new(rank, Suit::Clubs)
    }

    fn table(n: usize) -> BjProtocolState {
        let mut st = BjProtocolState::new();
        st.apply(&BjAction::Table {
            players: (0..n).map(|i| format!("did:example:p{}", i)).collect(),
            starting_chips: 1000,
            min_bet: 10,
        })
        .unwrap();
        st
    }

    /// Skip the crypto stages: wagers posted, initial deal already "done" by
    /// writing cards straight into the rules core.
    fn dealt_state(
        wagers: &[(usize, u64)],
        hands: &[(usize, Rank, Rank)],
        upcard: Rank,
    ) -> BjProtocolState {
        let mut st = table(3);
        for &(pid, amt) in wagers {
            st.game.apply_wager(pid, amt).unwrap();
        }
        for &(pid, a, b) in hands {
            st.game.deal_to_hand(pid, 0, c(a));
            st.game.deal_to_hand(pid, 0, c(b));
        }
        st.game.deal_to_banker(c(upcard));
        st.crypto.next_deck_position = 11;
        st
    }

    #[test]
    fn table_validates_player_count_and_min_bet() {
        let mut st = BjProtocolState::new();
        assert!(
            st.apply(&BjAction::Table {
                players: vec!["a".into()],
                starting_chips: 1000,
                min_bet: 10,
            })
            .is_err()
        );
        let mut st = BjProtocolState::new();
        assert!(
            st.apply(&BjAction::Table {
                players: (0..7).map(|i| i.to_string()).collect(),
                starting_chips: 1000,
                min_bet: 10,
            })
            .is_err()
        );
        let st = table(3);
        assert!(matches!(st.phase, BjPhase::CommitSeeds));
        assert_eq!(st.valid_actions().len(), 3);
    }

    #[test]
    fn commit_seeds_then_shuffle_then_lock_then_wagering() {
        crypto::init().ok();
        let mut st = table(2);
        for pid in 0..2 {
            st.apply(&BjAction::CommitSeed {
                player_id: pid,
                commitment: crypto::blake2b(format!("seed{}", pid).as_bytes()).unwrap(),
            })
            .unwrap();
        }
        assert!(matches!(st.phase, BjPhase::Shuffle { next_player: 0 }));
        assert_eq!(st.crypto.deck.len(), 52);

        let deck = st.crypto.deck.clone();
        st.apply(&BjAction::ShuffleDeck {
            player_id: 0,
            deck: deck.clone(),
        })
        .unwrap();
        assert!(matches!(st.phase, BjPhase::Shuffle { next_player: 1 }));
        st.apply(&BjAction::ShuffleDeck {
            player_id: 1,
            deck: deck.clone(),
        })
        .unwrap();
        assert!(matches!(st.phase, BjPhase::Lock { next_player: 0 }));
        st.apply(&BjAction::LockDeck {
            player_id: 0,
            deck: deck.clone(),
        })
        .unwrap();
        st.apply(&BjAction::LockDeck { player_id: 1, deck })
            .unwrap();
        // Banker is seat 0; the only bettor (seat 1) wagers.
        assert!(matches!(st.phase, BjPhase::Wagering { action_on: 1 }));
        match &st.valid_actions()[..] {
            [
                BjValidAction {
                    player_id: 1,
                    kind: BjValidActionKind::Wager { min: 10, max: 1000 },
                },
            ] => {}
            other => panic!("unexpected valid actions: {:?}", other),
        }
    }

    #[test]
    fn wagering_moves_to_initial_deal_in_order() {
        let mut st = table(3);
        // Manually open wagering (skipping crypto).
        st.phase = BjPhase::Wagering { action_on: 1 };
        assert!(
            st.apply(&BjAction::Wager {
                player_id: 2,
                amount: 10
            })
            .is_err()
        );
        st.apply(&BjAction::Wager {
            player_id: 1,
            amount: 10,
        })
        .unwrap();
        assert!(matches!(st.phase, BjPhase::Wagering { action_on: 2 }));
        st.apply(&BjAction::Wager {
            player_id: 2,
            amount: 25,
        })
        .unwrap();
        // Initial deal: bettor 1, bettor 2, banker, bettor 1, bettor 2.
        assert_eq!(
            st.deal_queue,
            vec![
                DealTarget::PlayerHand { player: 1, hand: 0 },
                DealTarget::PlayerHand { player: 2, hand: 0 },
                DealTarget::Banker,
                DealTarget::PlayerHand { player: 1, hand: 0 },
                DealTarget::PlayerHand { player: 2, hand: 0 },
            ]
        );
        assert!(matches!(
            st.phase,
            BjPhase::Dealing {
                target: DealTarget::PlayerHand { player: 1, hand: 0 },
                deck_position: 0
            }
        ));
    }

    #[test]
    fn insurance_offered_only_on_ace_upcard() {
        let mut st = dealt_state(
            &[(1, 100), (2, 100)],
            &[(1, Rank::Ten, Rank::Nine), (2, Rank::Ten, Rank::Eight)],
            Rank::Ace,
        );
        st.advance_round();
        assert!(matches!(st.phase, BjPhase::Insurance { action_on: 1 }));
        st.apply(&BjAction::Insurance {
            player_id: 1,
            take: true,
        })
        .unwrap();
        assert!(matches!(st.phase, BjPhase::Insurance { action_on: 2 }));
        st.apply(&BjAction::Insurance {
            player_id: 2,
            take: false,
        })
        .unwrap();
        // Insurance resolved — on to the first decision.
        assert!(matches!(
            st.phase,
            BjPhase::PlayerTurn {
                action_on: 1,
                hand: 0
            }
        ));
        assert!(st.insurance_done);
    }

    #[test]
    fn no_insurance_phase_without_ace() {
        let mut st = dealt_state(
            &[(1, 100), (2, 100)],
            &[(1, Rank::Ten, Rank::Nine), (2, Rank::Ten, Rank::Eight)],
            Rank::King,
        );
        st.advance_round();
        assert!(matches!(
            st.phase,
            BjPhase::PlayerTurn {
                action_on: 1,
                hand: 0
            }
        ));
    }

    #[test]
    fn hit_schedules_a_deal_and_returns_to_the_same_hand() {
        let mut st = dealt_state(
            &[(1, 100), (2, 100)],
            &[(1, Rank::Five, Rank::Nine), (2, Rank::Ten, Rank::Eight)],
            Rank::King,
        );
        st.advance_round();
        st.apply(&BjAction::Decision {
            player_id: 1,
            decision: Decision::Hit,
        })
        .unwrap();
        assert!(matches!(
            st.phase,
            BjPhase::Dealing {
                target: DealTarget::PlayerHand { player: 1, hand: 0 },
                deck_position: 11
            }
        ));
        // Simulate the dealt card arriving (3 → total 17, still this player).
        st.game.deal_to_hand(1, 0, c(Rank::Three));
        st.deal_queue.remove(0);
        st.advance_round();
        assert!(matches!(
            st.phase,
            BjPhase::PlayerTurn {
                action_on: 1,
                hand: 0
            }
        ));
    }

    #[test]
    fn stand_passes_to_next_bettor_then_banker_draws_then_complete() {
        let mut st = dealt_state(
            &[(1, 100), (2, 100)],
            &[(1, Rank::Ten, Rank::Nine), (2, Rank::Ten, Rank::Eight)],
            Rank::King,
        );
        st.advance_round();
        st.apply(&BjAction::Decision {
            player_id: 1,
            decision: Decision::Stand,
        })
        .unwrap();
        assert!(matches!(
            st.phase,
            BjPhase::PlayerTurn {
                action_on: 2,
                hand: 0
            }
        ));
        st.apply(&BjAction::Decision {
            player_id: 2,
            decision: Decision::Stand,
        })
        .unwrap();
        // Banker has 10 — must draw.
        assert!(matches!(
            st.phase,
            BjPhase::Dealing {
                target: DealTarget::Banker,
                ..
            }
        ));
        st.game.deal_to_banker(c(Rank::Seven)); // 17 — stands
        st.deal_queue.remove(0);
        st.advance_round();
        assert!(matches!(st.phase, BjPhase::Complete));
        let result = st.last_round_result.as_ref().unwrap();
        assert_eq!(result.banker.total, 17);
        // 19 and 18 both beat 17.
        assert_eq!(st.game.players[1].chips, 1100);
        assert_eq!(st.game.players[2].chips, 1100);
        assert_eq!(st.game.players[0].chips, 800);
    }

    #[test]
    fn deck_exhaustion_auto_stands_and_banker_stands() {
        let mut st = dealt_state(
            &[(1, 100), (2, 100)],
            &[(1, Rank::Five, Rank::Nine), (2, Rank::Ten, Rank::Eight)],
            Rank::King,
        );
        st.crypto.next_deck_position = DECK_SIZE; // no cards left
        st.advance_round();
        st.apply(&BjAction::Decision {
            player_id: 1,
            decision: Decision::Hit,
        })
        .unwrap();
        // No card to deal: the hand auto-stands, play moves on, the banker
        // can't draw either, and the round settles on current totals.
        assert!(matches!(st.phase, BjPhase::PlayerTurn { action_on: 2, .. }));
        st.apply(&BjAction::Decision {
            player_id: 2,
            decision: Decision::Stand,
        })
        .unwrap();
        assert!(matches!(st.phase, BjPhase::Complete));
        let result = st.last_round_result.as_ref().unwrap();
        assert_eq!(result.banker.total, 10);
        // 14 and 18 both beat the banker's 10.
        assert_eq!(st.game.players[1].chips, 1100);
        assert_eq!(st.game.players[2].chips, 1100);
    }

    #[test]
    fn next_round_rotates_banker_and_resets() {
        crypto::init().ok();
        let mut st = dealt_state(
            &[(1, 100), (2, 100)],
            &[(1, Rank::Ten, Rank::Nine), (2, Rank::Ten, Rank::Eight)],
            Rank::King,
        );
        st.advance_round();
        st.apply(&BjAction::Decision {
            player_id: 1,
            decision: Decision::Stand,
        })
        .unwrap();
        st.apply(&BjAction::Decision {
            player_id: 2,
            decision: Decision::Stand,
        })
        .unwrap();
        st.game.deal_to_banker(c(Rank::Seven));
        st.deal_queue.remove(0);
        st.advance_round();
        assert!(matches!(st.phase, BjPhase::Complete));

        assert!(!st.game_over());
        st.start_next_round();
        assert_eq!(st.game.banker, 1);
        assert_eq!(st.hand_index, 1);
        assert!(matches!(st.phase, BjPhase::CommitSeeds));
        assert!(st.last_round_result.is_none());
        assert!(!st.insurance_done);
        assert!(st.crypto.seed_commitments.iter().all(|c| c.is_none()));
        assert_eq!(st.crypto.next_deck_position, 0);
    }

    #[test]
    fn verify_seed_after_complete() {
        crypto::init().ok();
        let mut st = table(2);
        st.phase = BjPhase::Complete;
        let seed = b"bj_seed".to_vec();
        st.crypto.seed_commitments[0] = Some(crypto::blake2b(&seed).unwrap());
        st.apply(&BjAction::VerifySeed {
            player_id: 0,
            seed: seed.clone(),
        })
        .unwrap();
        assert!(st.crypto.seeds_verified[0]);
        // Wrong seed rejected.
        st.crypto.seed_commitments[1] = Some(crypto::blake2b(b"real").unwrap());
        assert!(
            st.apply(&BjAction::VerifySeed {
                player_id: 1,
                seed: b"fake".to_vec(),
            })
            .is_err()
        );
    }

    #[test]
    fn out_of_phase_actions_rejected() {
        let mut st = table(3);
        assert!(
            st.apply(&BjAction::Wager {
                player_id: 1,
                amount: 10
            })
            .is_err()
        );
        assert!(
            st.apply(&BjAction::Decision {
                player_id: 1,
                decision: Decision::Hit
            })
            .is_err()
        );
        assert!(
            st.apply(&BjAction::VerifySeed {
                player_id: 0,
                seed: vec![1, 2, 3]
            })
            .is_err()
        );
    }
}
