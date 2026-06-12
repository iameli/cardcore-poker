//! Blackjack round state and rules: wagering, decisions, banker play, settlement.
//!
//! European no-hole-card (ENHC), rotating banker:
//! - One seat banks the round; every other live seat wagers against the
//!   banker's chip stack.
//! - Wagers, doubles, splits, and insurance are escrowed from chips the
//!   moment they're placed.
//! - The banker draws to 17 and stands on soft 17 (S17). A banker two-card 21
//!   takes doubles and splits (standard ENHC) but pushes against a bettor
//!   blackjack.
//! - Settlement is collect-then-pay in seat order: stake returns come from
//!   escrow, winnings are paid from the banker's stack and capped by it (the
//!   banker can go bankrupt mid-payout).

use serde::{Deserialize, Serialize};

use crate::blackjack::eval::{self, HandValue, hand_value};
use crate::card::{Card, Rank};
use crate::engine::PlayerId;

/// A bettor's choice for one hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Hit,
    Stand,
    Double,
    Split,
    Surrender,
}

/// One playable hand (a bettor has two after a split).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hand {
    pub cards: Vec<Card>,
    /// Chips escrowed on this hand (doubles bump it).
    pub wager: u64,
    pub doubled: bool,
    pub stood: bool,
    pub busted: bool,
    /// Hand came from splitting aces: it gets exactly one card, then stands.
    pub split_aces: bool,
}

impl Hand {
    fn new(wager: u64) -> Self {
        Self {
            cards: Vec::new(),
            wager,
            doubled: false,
            stood: false,
            busted: false,
            split_aces: false,
        }
    }

    /// A resolved hand needs no further cards or decisions.
    pub fn resolved(&self) -> bool {
        self.stood || self.busted
    }

    pub fn value(&self) -> HandValue {
        hand_value(&self.cards)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BjPlayer {
    pub chips: u64,
    pub eliminated: bool,
    /// The wager posted this round (already escrowed). 0 = not posted yet.
    pub wager: u64,
    /// Escrowed insurance cost (0 = none taken).
    pub insurance: u64,
    /// Whether this bettor has answered the insurance offer this round.
    pub insurance_decided: bool,
    pub hands: Vec<Hand>,
    pub surrendered: bool,
}

impl BjPlayer {
    fn new(chips: u64) -> Self {
        Self {
            chips,
            eliminated: false,
            wager: 0,
            insurance: 0,
            insurance_decided: false,
            hands: Vec::new(),
            surrendered: false,
        }
    }
}

/// What the rules need next from the bettors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnNeed {
    /// The hand needs a card before play continues (after a hit/double/split).
    Card { player: PlayerId, hand: usize },
    /// The hand awaits a hit/stand/double/split/surrender decision.
    Decision { player: PlayerId, hand: usize },
}

/// How a hand fared against the banker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Blackjack,
    Win,
    Push,
    Lose,
    Bust,
    Surrender,
}

impl Outcome {
    fn as_str(&self) -> &'static str {
        match self {
            Outcome::Blackjack => "blackjack",
            Outcome::Win => "win",
            Outcome::Push => "push",
            Outcome::Lose => "lose",
            Outcome::Bust => "bust",
            Outcome::Surrender => "surrender",
        }
    }
}

/// One hand's line in the round log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandLog {
    pub cards: Vec<String>,
    pub total: u8,
    pub wager: u64,
    /// "blackjack" | "win" | "push" | "lose" | "bust" | "surrender"
    pub outcome: String,
    /// Total chips this hand returned to the player (stake + winnings; for a
    /// surrender, the kept half already received at decision time).
    pub payout: u64,
}

/// One player's line in the round log (banker and eliminated seats included,
/// with no hands).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerRoundLog {
    pub seat: PlayerId,
    pub hands: Vec<HandLog>,
    /// Insurance cost escrowed (0 = none).
    pub insurance: u64,
    /// Chips received back from insurance (3x cost on a banker blackjack).
    pub insurance_payout: u64,
    pub surrendered: bool,
    pub chips_after: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankerLog {
    pub seat: PlayerId,
    pub cards: Vec<String>,
    pub total: u8,
    pub blackjack: bool,
    pub bust: bool,
}

/// Result of a completed round — what the UI logs so players can follow along.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundResult {
    pub round_index: u64,
    pub banker: BankerLog,
    pub players: Vec<PlayerRoundLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackjackGame {
    pub players: Vec<BjPlayer>,
    /// Seat banking this round. Rotates to the next live seat each round.
    pub banker: usize,
    pub banker_cards: Vec<Card>,
    pub min_bet: u64,
    /// A card owed to a hand (after a hit/double/split) before play continues.
    pub pending_card: Option<(PlayerId, usize)>,
}

impl BlackjackGame {
    pub fn new(num_players: usize, starting_chips: u64, min_bet: u64) -> Self {
        assert!(num_players == 0 || (num_players >= 2 && num_players <= 6));
        Self {
            players: (0..num_players)
                .map(|_| BjPlayer::new(starting_chips))
                .collect(),
            banker: 0,
            banker_cards: Vec::new(),
            min_bet,
            pending_card: None,
        }
    }

    pub fn num_players(&self) -> usize {
        self.players.len()
    }

    /// Seats still in the game (the cryptographic roster), in seat order.
    pub fn live_seats(&self) -> Vec<usize> {
        (0..self.num_players())
            .filter(|&i| !self.players[i].eliminated)
            .collect()
    }

    /// Live non-banker seats in turn order, starting left of the banker.
    pub fn bettor_seats(&self) -> Vec<usize> {
        let n = self.num_players();
        (1..n)
            .map(|i| (self.banker + i) % n)
            .filter(|&s| !self.players[s].eliminated)
            .collect()
    }

    /// Next seat after `seat` (cyclic) that is still in the game.
    /// Returns `seat` itself if no other live seat exists.
    pub fn next_live_seat(&self, seat: usize) -> usize {
        let n = self.num_players();
        for i in 1..=n {
            let idx = (seat + i) % n;
            if !self.players[idx].eliminated {
                return idx;
            }
        }
        seat
    }

    // --- Wagering ---

    /// Allowed wager range: short stacks may go all-in below the table
    /// minimum; nobody can wager more than they have.
    pub fn wager_bounds(&self, player_id: PlayerId) -> (u64, u64) {
        let chips = self.players[player_id].chips;
        (self.min_bet.min(chips), chips)
    }

    /// The bettor whose wager is up next (seat order, left of the banker).
    pub fn next_wagerer(&self) -> Option<PlayerId> {
        self.bettor_seats()
            .into_iter()
            .find(|&s| self.players[s].wager == 0)
    }

    pub fn apply_wager(&mut self, player_id: PlayerId, amount: u64) -> crate::Result<()> {
        if self.next_wagerer() != Some(player_id) {
            return Err(crate::Error::InvalidAction("not your turn to wager".into()));
        }
        let (min, max) = self.wager_bounds(player_id);
        if amount < min || amount > max {
            return Err(crate::Error::InvalidAction(format!(
                "wager must be between {} and {}",
                min, max
            )));
        }
        let p = &mut self.players[player_id];
        p.chips -= amount;
        p.wager = amount;
        p.hands.push(Hand::new(amount));
        Ok(())
    }

    // --- Dealing ---

    /// Add a dealt card to a bettor's hand and auto-resolve: bust above 21,
    /// auto-stand on 21 (incl. naturals), after a double's one card, and on a
    /// split ace's one card.
    pub fn deal_to_hand(&mut self, player_id: PlayerId, hand: usize, card: Card) {
        if self.pending_card == Some((player_id, hand)) {
            self.pending_card = None;
        }
        let h = &mut self.players[player_id].hands[hand];
        h.cards.push(card);
        let v = hand_value(&h.cards);
        if v.total > 21 {
            h.busted = true;
        } else if v.total == 21 {
            h.stood = true;
        } else if h.doubled {
            h.stood = true;
        } else if h.split_aces && h.cards.len() == 2 {
            h.stood = true;
        }
    }

    pub fn deal_to_banker(&mut self, card: Card) {
        self.banker_cards.push(card);
    }

    // --- Insurance ---

    /// Insurance is offered after the initial deal when the banker's upcard
    /// is an ace.
    pub fn insurance_offered(&self) -> bool {
        self.banker_cards.len() == 1 && self.banker_cards[0].rank == Rank::Ace
    }

    /// Insurance costs half the wager, rounded down.
    pub fn insurance_cost(&self, player_id: PlayerId) -> u64 {
        self.players[player_id].wager / 2
    }

    /// Insurance is unavailable when the cost rounds to zero or the bettor
    /// can't afford it.
    pub fn insurance_available(&self, player_id: PlayerId) -> bool {
        let cost = self.insurance_cost(player_id);
        cost >= 1 && self.players[player_id].chips >= cost
    }

    /// The bettor whose insurance answer is up next (eligible seats only).
    pub fn next_insurer(&self) -> Option<PlayerId> {
        self.bettor_seats()
            .into_iter()
            .find(|&s| !self.players[s].insurance_decided && self.insurance_available(s))
    }

    pub fn apply_insurance(&mut self, player_id: PlayerId, take: bool) -> crate::Result<()> {
        if self.next_insurer() != Some(player_id) {
            return Err(crate::Error::InvalidAction(
                "not your turn to answer insurance".into(),
            ));
        }
        let cost = self.insurance_cost(player_id);
        let p = &mut self.players[player_id];
        p.insurance_decided = true;
        if take {
            p.chips -= cost;
            p.insurance = cost;
        }
        Ok(())
    }

    // --- Player turns ---

    /// What the bettors need next, in seat order (split hand 0 before hand 1):
    /// a card owed to a hand, or a decision. `None` once every hand resolved.
    pub fn next_turn_need(&self) -> Option<TurnNeed> {
        if let Some((player, hand)) = self.pending_card {
            return Some(TurnNeed::Card { player, hand });
        }
        for pid in self.bettor_seats() {
            let p = &self.players[pid];
            if p.surrendered {
                continue;
            }
            for (i, h) in p.hands.iter().enumerate() {
                if h.resolved() {
                    continue;
                }
                if h.cards.len() < 2 {
                    // A split hand still waiting for its second card.
                    return Some(TurnNeed::Card {
                        player: pid,
                        hand: i,
                    });
                }
                return Some(TurnNeed::Decision {
                    player: pid,
                    hand: i,
                });
            }
        }
        None
    }

    /// Legal moves for a hand. Double, split, and surrender are only offered
    /// on the first decision (two cards, unsplit); double and split also
    /// require enough chips for the extra wager.
    pub fn legal_decisions(&self, player_id: PlayerId, hand: usize) -> Vec<Decision> {
        let p = &self.players[player_id];
        if p.surrendered {
            return vec![];
        }
        let Some(h) = p.hands.get(hand) else {
            return vec![];
        };
        if h.resolved() || h.cards.len() < 2 {
            return vec![];
        }
        let mut opts = vec![Decision::Hit, Decision::Stand];
        let first_decision = h.cards.len() == 2 && !h.doubled;
        let unsplit = p.hands.len() == 1;
        if first_decision && unsplit {
            if p.chips >= h.wager {
                opts.push(Decision::Double);
                if h.cards[0].rank == h.cards[1].rank {
                    opts.push(Decision::Split);
                }
            }
            opts.push(Decision::Surrender);
        }
        opts
    }

    /// Apply a bettor's decision. Returns `true` when a card must now be
    /// dealt to this player's hand `hand` (hit, double, or split).
    pub fn apply_decision(
        &mut self,
        player_id: PlayerId,
        hand: usize,
        decision: Decision,
    ) -> crate::Result<bool> {
        match self.next_turn_need() {
            Some(TurnNeed::Decision { player, hand: h }) if player == player_id && h == hand => {}
            _ => {
                return Err(crate::Error::InvalidAction("not your turn to act".into()));
            }
        }
        if !self.legal_decisions(player_id, hand).contains(&decision) {
            return Err(crate::Error::InvalidAction(format!(
                "{:?} is not a legal move for this hand",
                decision
            )));
        }
        match decision {
            Decision::Hit => {
                self.pending_card = Some((player_id, hand));
                Ok(true)
            }
            Decision::Stand => {
                self.players[player_id].hands[hand].stood = true;
                Ok(false)
            }
            Decision::Double => {
                let p = &mut self.players[player_id];
                let add = p.hands[hand].wager;
                p.chips -= add;
                p.hands[hand].wager += add;
                p.hands[hand].doubled = true;
                self.pending_card = Some((player_id, hand));
                Ok(true)
            }
            Decision::Split => {
                let p = &mut self.players[player_id];
                let h = &mut p.hands[0];
                let second = h.cards.pop().unwrap();
                let aces = h.cards[0].rank == Rank::Ace;
                h.split_aces = aces;
                let w = h.wager;
                p.chips -= w;
                p.hands.push(Hand {
                    cards: vec![second],
                    wager: w,
                    doubled: false,
                    stood: false,
                    busted: false,
                    split_aces: aces,
                });
                self.pending_card = Some((player_id, 0));
                Ok(true)
            }
            Decision::Surrender => {
                // Late surrender: forfeit half (rounded up, odd chip to the
                // banker) right away; the hand leaves the round.
                let w = self.players[player_id].hands[hand].wager;
                let forfeit = w.div_ceil(2);
                self.players[player_id].chips += w - forfeit;
                self.players[player_id].surrendered = true;
                self.players[player_id].hands[hand].stood = true;
                let banker = self.banker;
                self.players[banker].chips += forfeit;
                Ok(false)
            }
        }
    }

    /// Deck-exhaustion guard: stand the hand on its current total.
    pub fn force_stand(&mut self, player_id: PlayerId, hand: usize) {
        if self.pending_card == Some((player_id, hand)) {
            self.pending_card = None;
        }
        self.players[player_id].hands[hand].stood = true;
    }

    // --- Banker play ---

    pub fn banker_value(&self) -> HandValue {
        hand_value(&self.banker_cards)
    }

    /// S17: the banker draws until reaching 17, standing on soft 17.
    pub fn banker_must_draw(&self) -> bool {
        self.banker_value().total < 17
    }

    // --- Settlement ---

    /// Settle the round deterministically: the banker first collects every
    /// losing wager and lost insurance, then pays in seat order (insurance
    /// first, then hands in seat/hand order). Stake returns come from escrow;
    /// winnings come from the banker's stack, capped by what's left of it.
    pub fn settle(&mut self, round_index: u64) -> RoundResult {
        let banker_v = self.banker_value();
        let banker_blackjack = eval::is_blackjack(&self.banker_cards);
        let banker_bust = banker_v.total > 21;
        let banker = self.banker;
        let bettors = self.bettor_seats();

        // Outcomes (read-only pass).
        let mut outcomes: Vec<Vec<Outcome>> = vec![Vec::new(); self.num_players()];
        for &pid in &bettors {
            let p = &self.players[pid];
            let player_bj =
                p.hands.len() == 1 && !p.hands[0].doubled && eval::is_blackjack(&p.hands[0].cards);
            for h in &p.hands {
                let o = if p.surrendered {
                    Outcome::Surrender
                } else if h.busted {
                    Outcome::Bust
                } else if banker_blackjack {
                    if player_bj {
                        Outcome::Push
                    } else {
                        // ENHC: a banker blackjack takes the full committed
                        // amount, doubles and splits included.
                        Outcome::Lose
                    }
                } else if player_bj {
                    Outcome::Blackjack
                } else if banker_bust {
                    Outcome::Win
                } else {
                    let pv = hand_value(&h.cards);
                    if pv.total > banker_v.total {
                        Outcome::Win
                    } else if pv.total == banker_v.total {
                        Outcome::Push
                    } else {
                        Outcome::Lose
                    }
                };
                outcomes[pid].push(o);
            }
        }

        // Collect: losing wagers and lost insurance go to the banker.
        // (Surrendered forfeits were transferred at decision time.)
        for &pid in &bettors {
            if self.players[pid].insurance > 0 && !banker_blackjack {
                self.players[banker].chips += self.players[pid].insurance;
            }
            for (i, &o) in outcomes[pid].iter().enumerate() {
                if matches!(o, Outcome::Lose | Outcome::Bust) {
                    self.players[banker].chips += self.players[pid].hands[i].wager;
                }
            }
        }

        // Pay: in seat order, insurance first, then hands. Stakes return from
        // escrow; winnings are capped by the banker's remaining chips.
        let mut insurance_payouts: Vec<u64> = vec![0; self.num_players()];
        let mut hand_payouts: Vec<Vec<u64>> = vec![Vec::new(); self.num_players()];
        for &pid in &bettors {
            let ins = self.players[pid].insurance;
            if ins > 0 && banker_blackjack {
                let winnings = (2 * ins).min(self.players[banker].chips);
                self.players[banker].chips -= winnings;
                self.players[pid].chips += ins + winnings;
                insurance_payouts[pid] = ins + winnings;
            }
            for (i, &o) in outcomes[pid].iter().enumerate() {
                let wager = self.players[pid].hands[i].wager;
                let payout = match o {
                    Outcome::Blackjack | Outcome::Win => {
                        let due = match o {
                            Outcome::Blackjack => wager * 3 / 2,
                            _ => wager,
                        };
                        let winnings = due.min(self.players[banker].chips);
                        self.players[banker].chips -= winnings;
                        self.players[pid].chips += wager + winnings;
                        wager + winnings
                    }
                    Outcome::Push => {
                        self.players[pid].chips += wager;
                        wager
                    }
                    Outcome::Lose | Outcome::Bust => 0,
                    // Already received at decision time.
                    Outcome::Surrender => wager - wager.div_ceil(2),
                };
                hand_payouts[pid].push(payout);
            }
        }

        // Build the log.
        let players_log: Vec<PlayerRoundLog> = (0..self.num_players())
            .map(|pid| {
                let p = &self.players[pid];
                let hands = p
                    .hands
                    .iter()
                    .enumerate()
                    .map(|(i, h)| HandLog {
                        cards: h.cards.iter().map(|c| c.to_string()).collect(),
                        total: hand_value(&h.cards).total,
                        wager: h.wager,
                        outcome: outcomes[pid][i].as_str().to_string(),
                        payout: hand_payouts[pid].get(i).copied().unwrap_or(0),
                    })
                    .collect();
                PlayerRoundLog {
                    seat: pid,
                    hands,
                    insurance: p.insurance,
                    insurance_payout: insurance_payouts[pid],
                    surrendered: p.surrendered,
                    chips_after: p.chips,
                }
            })
            .collect();

        RoundResult {
            round_index,
            banker: BankerLog {
                seat: banker,
                cards: self.banker_cards.iter().map(|c| c.to_string()).collect(),
                total: banker_v.total,
                blackjack: banker_blackjack,
                bust: banker_bust,
            },
            players: players_log,
        }
    }

    // --- Rounds ---

    /// The game is over once at most one player has chips left.
    pub fn game_over(&self) -> bool {
        self.players.iter().filter(|p| p.chips > 0).count() <= 1
    }

    /// Begin the next round: bust broke players, rotate the banker to the
    /// next live seat, and clear per-round state. Seats (and thus the
    /// cryptographic roster) stay fixed — busted players sit out but still
    /// participate in shuffling/dealing.
    pub fn start_next_round(&mut self) {
        for p in &mut self.players {
            if p.chips == 0 {
                p.eliminated = true;
            }
        }
        self.banker = self.next_live_seat(self.banker);
        self.reset_round();
    }

    fn reset_round(&mut self) {
        self.banker_cards.clear();
        self.pending_card = None;
        for p in &mut self.players {
            p.wager = 0;
            p.insurance = 0;
            p.insurance_decided = false;
            p.hands.clear();
            p.surrendered = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::Suit;

    fn c(s: &str) -> Card {
        let mut chars = s.chars();
        let rank = match chars.next().unwrap() {
            '2' => Rank::Two,
            '3' => Rank::Three,
            '4' => Rank::Four,
            '5' => Rank::Five,
            '6' => Rank::Six,
            '7' => Rank::Seven,
            '8' => Rank::Eight,
            '9' => Rank::Nine,
            'T' => Rank::Ten,
            'J' => Rank::Jack,
            'Q' => Rank::Queen,
            'K' => Rank::King,
            'A' => Rank::Ace,
            other => panic!("bad rank {}", other),
        };
        let suit = match chars.next().unwrap() {
            'c' => Suit::Clubs,
            'd' => Suit::Diamonds,
            'h' => Suit::Hearts,
            's' => Suit::Spades,
            other => panic!("bad suit {}", other),
        };
        Card::new(rank, suit)
    }

    fn total_chips(g: &BlackjackGame) -> u64 {
        g.players.iter().map(|p| p.chips).sum()
    }

    /// 3 players, 1000 chips each, min bet 10, banker seat 0.
    fn wagered_game(wagers: &[(usize, u64)]) -> BlackjackGame {
        let mut g = BlackjackGame::new(3, 1000, 10);
        for &(pid, amt) in wagers {
            g.apply_wager(pid, amt).unwrap();
        }
        g
    }

    /// Deal two cards to a bettor's first hand.
    fn deal2(g: &mut BlackjackGame, pid: usize, a: &str, b: &str) {
        g.deal_to_hand(pid, 0, c(a));
        g.deal_to_hand(pid, 0, c(b));
    }

    // --- Wagering ---

    #[test]
    fn wagering_in_seat_order_with_bounds() {
        let mut g = BlackjackGame::new(3, 1000, 10);
        assert_eq!(g.next_wagerer(), Some(1));
        assert!(g.apply_wager(2, 10).is_err(), "out of turn");
        assert!(g.apply_wager(1, 9).is_err(), "below min");
        assert!(g.apply_wager(1, 1001).is_err(), "above chips");
        g.apply_wager(1, 50).unwrap();
        assert_eq!(g.players[1].chips, 950);
        assert_eq!(g.next_wagerer(), Some(2));
        g.apply_wager(2, 10).unwrap();
        assert_eq!(g.next_wagerer(), None);
    }

    #[test]
    fn short_stack_wagers_all_in_below_min() {
        let mut g = BlackjackGame::new(3, 1000, 10);
        g.players[1].chips = 4;
        assert_eq!(g.wager_bounds(1), (4, 4));
        assert!(g.apply_wager(1, 10).is_err());
        g.apply_wager(1, 4).unwrap();
        assert_eq!(g.players[1].chips, 0);
    }

    // --- Decision legality ---

    #[test]
    fn first_decision_offers_double_split_surrender() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        deal2(&mut g, 1, "8c", "8d");
        let opts = g.legal_decisions(1, 0);
        assert!(opts.contains(&Decision::Hit));
        assert!(opts.contains(&Decision::Stand));
        assert!(opts.contains(&Decision::Double));
        assert!(opts.contains(&Decision::Split));
        assert!(opts.contains(&Decision::Surrender));
        // No pair (and K/Q are different ranks even though both count 10).
        deal2(&mut g, 2, "Kc", "Qd");
        let opts = g.legal_decisions(2, 0);
        assert!(!opts.contains(&Decision::Split));
        assert!(opts.contains(&Decision::Double));
    }

    #[test]
    fn after_hit_only_hit_or_stand() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        deal2(&mut g, 1, "5c", "8d");
        deal2(&mut g, 2, "Th", "7d");
        assert!(g.apply_decision(1, 0, Decision::Hit).unwrap());
        g.deal_to_hand(1, 0, c("2s"));
        assert_eq!(
            g.legal_decisions(1, 0),
            vec![Decision::Hit, Decision::Stand]
        );
    }

    #[test]
    fn double_and_split_require_chips() {
        let mut g = BlackjackGame::new(2, 1000, 10);
        g.players[1].chips = 100;
        g.apply_wager(1, 100).unwrap(); // all-in wager — nothing left to double
        deal2(&mut g, 1, "8c", "8d");
        let opts = g.legal_decisions(1, 0);
        assert!(!opts.contains(&Decision::Double));
        assert!(!opts.contains(&Decision::Split));
        assert!(opts.contains(&Decision::Surrender));
    }

    #[test]
    fn split_plays_both_hands_no_resplit_no_double_after_split() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        deal2(&mut g, 1, "8c", "8d");
        deal2(&mut g, 2, "Th", "7d");
        assert!(g.apply_decision(1, 0, Decision::Split).unwrap());
        assert_eq!(g.players[1].chips, 800, "second wager escrowed");
        assert_eq!(g.players[1].hands.len(), 2);
        assert_eq!(
            g.next_turn_need(),
            Some(TurnNeed::Card { player: 1, hand: 0 })
        );
        g.deal_to_hand(1, 0, c("8h")); // pairs again — still no resplit
        let opts = g.legal_decisions(1, 0);
        assert!(!opts.contains(&Decision::Split));
        assert!(!opts.contains(&Decision::Double));
        assert!(!opts.contains(&Decision::Surrender));
        g.apply_decision(1, 0, Decision::Stand).unwrap();
        assert_eq!(
            g.next_turn_need(),
            Some(TurnNeed::Card { player: 1, hand: 1 })
        );
        g.deal_to_hand(1, 1, c("3c"));
        assert_eq!(
            g.next_turn_need(),
            Some(TurnNeed::Decision { player: 1, hand: 1 })
        );
        g.apply_decision(1, 1, Decision::Stand).unwrap();
        assert_eq!(
            g.next_turn_need(),
            Some(TurnNeed::Decision { player: 2, hand: 0 })
        );
    }

    #[test]
    fn split_aces_get_one_card_each_and_stand() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        deal2(&mut g, 1, "Ac", "Ad");
        deal2(&mut g, 2, "Th", "7d");
        g.apply_decision(1, 0, Decision::Split).unwrap();
        g.deal_to_hand(1, 0, c("9c"));
        assert!(g.players[1].hands[0].stood, "split ace auto-stands");
        assert_eq!(
            g.next_turn_need(),
            Some(TurnNeed::Card { player: 1, hand: 1 })
        );
        g.deal_to_hand(1, 1, c("Kc"));
        assert!(g.players[1].hands[1].stood);
        assert_eq!(
            g.next_turn_need(),
            Some(TurnNeed::Decision { player: 2, hand: 0 })
        );
    }

    #[test]
    fn twenty_one_auto_stands_and_bust_resolves() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        deal2(&mut g, 1, "Ac", "Kd"); // natural
        assert!(g.players[1].hands[0].stood);
        deal2(&mut g, 2, "Tc", "9d");
        g.apply_decision(2, 0, Decision::Hit).unwrap();
        g.deal_to_hand(2, 0, c("5c"));
        assert!(g.players[2].hands[0].busted);
        assert_eq!(g.next_turn_need(), None);
    }

    #[test]
    fn surrender_first_decision_only_forfeits_ceil_half() {
        let mut g = wagered_game(&[(1, 11), (2, 10)]);
        deal2(&mut g, 1, "Tc", "6d");
        deal2(&mut g, 2, "Th", "7d");
        g.apply_decision(1, 0, Decision::Surrender).unwrap();
        // Odd wager 11: forfeit ceil(11/2)=6 to the banker, keep 5.
        assert_eq!(g.players[0].chips, 1006);
        assert_eq!(g.players[1].chips, 1000 - 11 + 5);
        assert!(g.players[1].surrendered);
        // After a hit, surrender (and double) are gone.
        g.apply_decision(2, 0, Decision::Hit).unwrap();
        g.deal_to_hand(2, 0, c("2s"));
        assert!(!g.legal_decisions(2, 0).contains(&Decision::Surrender));
    }

    #[test]
    fn out_of_turn_and_illegal_decisions_rejected() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        deal2(&mut g, 1, "5c", "8d");
        deal2(&mut g, 2, "Th", "7d");
        assert!(
            g.apply_decision(2, 0, Decision::Hit).is_err(),
            "not 2's turn"
        );
        assert!(
            g.apply_decision(1, 0, Decision::Split).is_err(),
            "not a pair"
        );
        g.apply_decision(1, 0, Decision::Hit).unwrap();
        assert!(
            g.apply_decision(1, 0, Decision::Hit).is_err(),
            "card still pending"
        );
    }

    // --- Insurance ---

    #[test]
    fn insurance_offer_cost_and_eligibility() {
        let mut g = BlackjackGame::new(3, 1000, 10);
        g.players[2].chips = 1;
        g.apply_wager(1, 100).unwrap();
        g.apply_wager(2, 1).unwrap(); // all-in 1: floor(1/2)=0 → ineligible
        deal2(&mut g, 1, "Tc", "6d");
        deal2(&mut g, 2, "Th", "7d");
        g.deal_to_banker(c("As"));
        assert!(g.insurance_offered());
        assert_eq!(g.insurance_cost(1), 50);
        assert!(g.insurance_available(1));
        assert!(!g.insurance_available(2));
        assert_eq!(g.next_insurer(), Some(1));
        assert!(g.apply_insurance(2, true).is_err());
        g.apply_insurance(1, true).unwrap();
        assert_eq!(g.players[1].chips, 850);
        assert_eq!(g.players[1].insurance, 50);
        assert_eq!(g.next_insurer(), None);
    }

    #[test]
    fn no_insurance_without_ace_upcard() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        g.deal_to_banker(c("Ks"));
        assert!(!g.insurance_offered());
    }

    // --- Settlement ---

    #[test]
    fn settle_win_push_lose_at_1_to_1() {
        let mut g = wagered_game(&[(1, 100), (2, 50)]);
        deal2(&mut g, 1, "Tc", "9d"); // 19: beats banker 18
        deal2(&mut g, 2, "Th", "8d"); // 18: pushes
        g.apply_decision(1, 0, Decision::Stand).unwrap();
        g.apply_decision(2, 0, Decision::Stand).unwrap();
        g.deal_to_banker(c("Ts"));
        g.deal_to_banker(c("8s")); // 18, stands
        assert!(!g.banker_must_draw());
        let r = g.settle(0);
        assert_eq!(g.players[1].chips, 1100);
        assert_eq!(g.players[2].chips, 1000);
        assert_eq!(g.players[0].chips, 900);
        assert_eq!(total_chips(&g), 3000);
        assert_eq!(r.players[1].hands[0].outcome, "win");
        assert_eq!(r.players[1].hands[0].payout, 200);
        assert_eq!(r.players[2].hands[0].outcome, "push");
        assert_eq!(r.banker.total, 18);
    }

    #[test]
    fn settle_blackjack_pays_3_to_2_floored() {
        let mut g = wagered_game(&[(1, 25), (2, 10)]);
        deal2(&mut g, 1, "Ac", "Kd"); // natural
        deal2(&mut g, 2, "Th", "9d");
        g.apply_decision(2, 0, Decision::Stand).unwrap();
        g.deal_to_banker(c("Ts"));
        g.deal_to_banker(c("8s"));
        let r = g.settle(0);
        // floor(1.5 * 25) = 37 winnings + 25 stake.
        assert_eq!(g.players[1].chips, 1037);
        assert_eq!(r.players[1].hands[0].outcome, "blackjack");
        assert_eq!(r.players[1].hands[0].payout, 62);
        assert_eq!(g.players[2].chips, 1010); // 19 beats 18
        assert_eq!(g.players[0].chips, 953);
        assert_eq!(total_chips(&g), 3000);
    }

    #[test]
    fn settle_bust_loses_even_when_banker_busts() {
        let mut g = wagered_game(&[(1, 100), (2, 50)]);
        deal2(&mut g, 1, "Tc", "9d");
        deal2(&mut g, 2, "Th", "8d"); // stands on 18
        g.apply_decision(1, 0, Decision::Hit).unwrap();
        g.deal_to_hand(1, 0, c("5c")); // 24 — bust
        g.apply_decision(2, 0, Decision::Stand).unwrap();
        g.deal_to_banker(c("Ts"));
        g.deal_to_banker(c("6s")); // 16 — must draw
        assert!(g.banker_must_draw());
        g.deal_to_banker(c("Qs")); // 26 — bust
        let r = g.settle(0);
        assert_eq!(r.players[1].hands[0].outcome, "bust");
        assert_eq!(g.players[1].chips, 900, "bust loses before banker bust");
        assert_eq!(g.players[2].chips, 1050, "stander beats busted banker");
        assert_eq!(g.players[0].chips, 1050);
        assert_eq!(total_chips(&g), 3000);
        assert!(r.banker.bust);
    }

    #[test]
    fn settle_banker_blackjack_takes_double_pushes_bettor_blackjack() {
        let mut g = wagered_game(&[(1, 100), (2, 50)]);
        deal2(&mut g, 1, "5c", "6d"); // 11 — double
        deal2(&mut g, 2, "Ac", "Kd"); // natural
        g.apply_decision(1, 0, Decision::Double).unwrap();
        g.deal_to_hand(1, 0, c("9c")); // 20, auto-stood
        g.deal_to_banker(c("As"));
        g.deal_to_banker(c("Ks")); // banker blackjack
        let r = g.settle(0);
        assert!(r.banker.blackjack);
        // ENHC: the doubled 200 is lost in full.
        assert_eq!(g.players[1].chips, 800);
        assert_eq!(r.players[1].hands[0].outcome, "lose");
        // A bettor blackjack pushes against a banker blackjack.
        assert_eq!(g.players[2].chips, 1000);
        assert_eq!(r.players[2].hands[0].outcome, "push");
        assert_eq!(g.players[0].chips, 1200);
        assert_eq!(total_chips(&g), 3000);
    }

    #[test]
    fn settle_insurance_pays_2_to_1_on_banker_blackjack() {
        let mut g = wagered_game(&[(1, 100), (2, 100)]);
        deal2(&mut g, 1, "Tc", "9d");
        deal2(&mut g, 2, "Th", "8d");
        g.deal_to_banker(c("As"));
        assert!(g.insurance_offered());
        g.apply_insurance(1, true).unwrap(); // cost 50
        g.apply_insurance(2, false).unwrap();
        g.apply_decision(1, 0, Decision::Stand).unwrap();
        g.apply_decision(2, 0, Decision::Stand).unwrap();
        g.deal_to_banker(c("Ks")); // banker blackjack
        let r = g.settle(0);
        // Hand loses 100, insurance returns 150: net back to 1000.
        assert_eq!(g.players[1].chips, 1000);
        assert_eq!(r.players[1].insurance_payout, 150);
        assert_eq!(g.players[2].chips, 900);
        assert_eq!(g.players[0].chips, 1100);
        assert_eq!(total_chips(&g), 3000);
    }

    #[test]
    fn settle_insurance_lost_without_banker_blackjack() {
        let mut g = wagered_game(&[(1, 100), (2, 100)]);
        deal2(&mut g, 1, "Tc", "9d");
        deal2(&mut g, 2, "Th", "8d");
        g.deal_to_banker(c("As"));
        g.apply_insurance(1, true).unwrap();
        g.apply_insurance(2, false).unwrap();
        g.apply_decision(1, 0, Decision::Stand).unwrap();
        g.apply_decision(2, 0, Decision::Stand).unwrap();
        g.deal_to_banker(c("6s")); // soft 17 — S17 stands
        assert!(!g.banker_must_draw());
        let r = g.settle(0);
        // 19 and 18 both beat 17; insurance 50 lost to the banker.
        assert_eq!(g.players[1].chips, 1050);
        assert_eq!(r.players[1].insurance_payout, 0);
        assert_eq!(g.players[2].chips, 1100);
        assert_eq!(g.players[0].chips, 850);
        assert_eq!(total_chips(&g), 3000);
    }

    #[test]
    fn settle_caps_payouts_at_banker_bankruptcy_in_seat_order() {
        let mut g = BlackjackGame::new(3, 1000, 10);
        g.players[0].chips = 30; // poor banker
        g.apply_wager(1, 100).unwrap();
        g.apply_wager(2, 100).unwrap();
        deal2(&mut g, 1, "Tc", "9d");
        deal2(&mut g, 2, "Td", "9h");
        g.apply_decision(1, 0, Decision::Stand).unwrap();
        g.apply_decision(2, 0, Decision::Stand).unwrap();
        g.deal_to_banker(c("Ts"));
        g.deal_to_banker(c("8s")); // 18 — both 19s win
        let r = g.settle(0);
        // Seat order from the banker: player 1 first (gets the 30), then 2.
        assert_eq!(g.players[1].chips, 900 + 100 + 30);
        assert_eq!(g.players[2].chips, 900 + 100);
        assert_eq!(g.players[0].chips, 0);
        assert_eq!(r.players[1].hands[0].payout, 130);
        assert_eq!(r.players[2].hands[0].payout, 100);
        assert_eq!(total_chips(&g), 2030);
    }

    #[test]
    fn split_hands_settle_independently() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        deal2(&mut g, 1, "8c", "8d");
        deal2(&mut g, 2, "Th", "7d");
        g.apply_decision(1, 0, Decision::Split).unwrap();
        g.deal_to_hand(1, 0, c("Qc")); // 18
        g.apply_decision(1, 0, Decision::Stand).unwrap();
        g.deal_to_hand(1, 1, c("3c")); // 11
        g.apply_decision(1, 1, Decision::Hit).unwrap();
        g.deal_to_hand(1, 1, c("Tc")); // 21 — auto-stands, but not a blackjack
        g.apply_decision(2, 0, Decision::Stand).unwrap();
        g.deal_to_banker(c("Ts"));
        g.deal_to_banker(c("8s")); // 18
        let r = g.settle(0);
        // Hand 0 pushes (100 back); hand 1 wins 1:1 (not 3:2 — split 21).
        assert_eq!(r.players[1].hands[0].outcome, "push");
        assert_eq!(r.players[1].hands[1].outcome, "win");
        assert_eq!(g.players[1].chips, 800 + 100 + 200);
        // Player 2's 17 loses to 18.
        assert_eq!(g.players[2].chips, 990);
        assert_eq!(g.players[0].chips, 910);
        assert_eq!(total_chips(&g), 3000);
    }

    #[test]
    fn settle_surrendered_player_only_logs_already_paid_half() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        deal2(&mut g, 1, "Tc", "6d");
        deal2(&mut g, 2, "Th", "7d");
        g.apply_decision(1, 0, Decision::Surrender).unwrap();
        g.apply_decision(2, 0, Decision::Stand).unwrap();
        g.deal_to_banker(c("As"));
        g.deal_to_banker(c("Ks")); // banker blackjack — surrender already resolved
        let r = g.settle(0);
        assert_eq!(r.players[1].hands[0].outcome, "surrender");
        assert_eq!(r.players[1].hands[0].payout, 50);
        assert_eq!(g.players[1].chips, 950, "kept half, no further loss");
        assert_eq!(g.players[2].chips, 990);
        assert_eq!(g.players[0].chips, 1060);
        assert_eq!(total_chips(&g), 3000);
    }

    // --- Rounds ---

    #[test]
    fn heads_up_round_works() {
        let mut g = BlackjackGame::new(2, 1000, 10);
        assert_eq!(g.bettor_seats(), vec![1]);
        g.apply_wager(1, 10).unwrap();
        deal2(&mut g, 1, "Tc", "9d");
        g.apply_decision(1, 0, Decision::Stand).unwrap();
        g.deal_to_banker(c("Ts"));
        g.deal_to_banker(c("7s"));
        g.settle(0);
        assert_eq!(g.players[1].chips, 1010);
        assert_eq!(g.players[0].chips, 990);
    }

    #[test]
    fn rotation_skips_eliminated_and_detects_game_over() {
        let mut g = BlackjackGame::new(3, 1000, 10);
        g.players[1].chips = 0;
        g.start_next_round();
        assert!(g.players[1].eliminated);
        assert_eq!(g.banker, 2, "banker skips the eliminated seat");
        assert_eq!(g.bettor_seats(), vec![0]);
        assert!(!g.game_over());
        g.players[0].chips = 0;
        g.start_next_round();
        assert!(g.game_over());
    }

    #[test]
    fn next_round_resets_per_round_state() {
        let mut g = wagered_game(&[(1, 100), (2, 10)]);
        deal2(&mut g, 1, "Tc", "6d");
        deal2(&mut g, 2, "Th", "7d");
        g.deal_to_banker(c("As"));
        g.apply_insurance(1, true).unwrap();
        g.start_next_round();
        assert_eq!(g.banker, 1);
        for p in &g.players {
            assert_eq!(p.wager, 0);
            assert_eq!(p.insurance, 0);
            assert!(!p.insurance_decided);
            assert!(p.hands.is_empty());
            assert!(!p.surrendered);
        }
        assert!(g.banker_cards.is_empty());
        assert_eq!(g.next_wagerer(), Some(2), "first bettor left of new banker");
    }
}
