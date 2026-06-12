//! Blackjack hand arithmetic: totals, soft hands, blackjack and bust checks.

use crate::card::{Card, Rank};

/// The value of a blackjack hand. `soft` means an ace is currently counted
/// as 11 (so the hand can't bust on the next card).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandValue {
    pub total: u8,
    pub soft: bool,
}

/// Blackjack value of a single card: 2–9 at face value, ten/face cards 10,
/// ace 1 (promotion to 11 happens in `hand_value`).
fn card_value(rank: Rank) -> u8 {
    match rank {
        Rank::Two => 2,
        Rank::Three => 3,
        Rank::Four => 4,
        Rank::Five => 5,
        Rank::Six => 6,
        Rank::Seven => 7,
        Rank::Eight => 8,
        Rank::Nine => 9,
        Rank::Ten | Rank::Jack | Rank::Queen | Rank::King => 10,
        Rank::Ace => 1,
    }
}

/// Best blackjack total for a hand: aces count 1, with one promoted to 11
/// when that doesn't bust (a "soft" total).
pub fn hand_value(cards: &[Card]) -> HandValue {
    let mut total: u8 = 0;
    let mut aces = 0;
    for c in cards {
        total += card_value(c.rank);
        if c.rank == Rank::Ace {
            aces += 1;
        }
    }
    if aces > 0 && total + 10 <= 21 {
        HandValue {
            total: total + 10,
            soft: true,
        }
    } else {
        HandValue { total, soft: false }
    }
}

/// A natural: exactly two cards totaling 21. Whether those two cards came
/// from a split (which downgrades 21 to a plain 21) is the game's concern.
pub fn is_blackjack(cards: &[Card]) -> bool {
    cards.len() == 2 && hand_value(cards).total == 21
}

/// Hand is bust (best total above 21).
pub fn is_bust(cards: &[Card]) -> bool {
    hand_value(cards).total > 21
}

/// Soft 17 (e.g. A+6): relevant because the banker stands on it (S17).
pub fn is_soft_17(cards: &[Card]) -> bool {
    let v = hand_value(cards);
    v.total == 17 && v.soft
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::Suit;

    fn cards(ranks: &[Rank]) -> Vec<Card> {
        ranks.iter().map(|&r| Card::new(r, Suit::Clubs)).collect()
    }

    #[test]
    fn hard_totals() {
        assert_eq!(
            hand_value(&cards(&[Rank::Ten, Rank::Seven])),
            HandValue {
                total: 17,
                soft: false
            }
        );
        assert_eq!(
            hand_value(&cards(&[Rank::King, Rank::Queen])),
            HandValue {
                total: 20,
                soft: false
            }
        );
    }

    #[test]
    fn soft_totals_promote_one_ace() {
        // A+6 = soft 17
        let v = hand_value(&cards(&[Rank::Ace, Rank::Six]));
        assert_eq!(
            v,
            HandValue {
                total: 17,
                soft: true
            }
        );
        // A+A = soft 12 (one ace as 11, one as 1)
        let v = hand_value(&cards(&[Rank::Ace, Rank::Ace]));
        assert_eq!(
            v,
            HandValue {
                total: 12,
                soft: true
            }
        );
        // A+A+9 = soft 21
        let v = hand_value(&cards(&[Rank::Ace, Rank::Ace, Rank::Nine]));
        assert_eq!(
            v,
            HandValue {
                total: 21,
                soft: true
            }
        );
    }

    #[test]
    fn ace_demotes_to_hard_when_needed() {
        // A+6+10 = hard 17 (ace back to 1)
        let v = hand_value(&cards(&[Rank::Ace, Rank::Six, Rank::Ten]));
        assert_eq!(
            v,
            HandValue {
                total: 17,
                soft: false
            }
        );
    }

    #[test]
    fn blackjack_is_two_card_21_only() {
        assert!(is_blackjack(&cards(&[Rank::Ace, Rank::King])));
        assert!(is_blackjack(&cards(&[Rank::Ten, Rank::Ace])));
        // 21 in three cards is not a blackjack
        assert!(!is_blackjack(&cards(&[
            Rank::Seven,
            Rank::Seven,
            Rank::Seven
        ])));
        assert!(!is_blackjack(&cards(&[Rank::Ten, Rank::Nine])));
    }

    #[test]
    fn bust_detection() {
        assert!(is_bust(&cards(&[Rank::Ten, Rank::Nine, Rank::Five])));
        assert!(!is_bust(&cards(&[Rank::Ten, Rank::Nine, Rank::Two])));
        // Aces save the hand from busting
        assert!(!is_bust(&cards(&[Rank::Ace, Rank::Nine, Rank::Five])));
    }

    #[test]
    fn soft_17_detection() {
        assert!(is_soft_17(&cards(&[Rank::Ace, Rank::Six])));
        assert!(is_soft_17(&cards(&[Rank::Ace, Rank::Two, Rank::Four])));
        // Hard 17 is not soft
        assert!(!is_soft_17(&cards(&[Rank::Ten, Rank::Seven])));
        // A+6+10 is hard 17
        assert!(!is_soft_17(&cards(&[Rank::Ace, Rank::Six, Rank::Ten])));
    }
}
