//! Poker hand evaluation for Texas Hold'em.
//!
//! Evaluates the best 5-card hand from 7 cards (2 hole + 5 community).
//! Uses a straightforward approach: enumerate all C(7,5)=21 combinations,
//! classify each, and pick the best.

use crate::card::{Card, Rank};
use std::cmp::Ordering;
use std::fmt;

/// Poker hand ranking, ordered from worst to best.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HandRank {
    HighCard,
    OnePair,
    TwoPair,
    ThreeOfAKind,
    Straight,
    Flush,
    FullHouse,
    FourOfAKind,
    StraightFlush,
}

impl fmt::Display for HandRank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandRank::HighCard => write!(f, "High Card"),
            HandRank::OnePair => write!(f, "One Pair"),
            HandRank::TwoPair => write!(f, "Two Pair"),
            HandRank::ThreeOfAKind => write!(f, "Three of a Kind"),
            HandRank::Straight => write!(f, "Straight"),
            HandRank::Flush => write!(f, "Flush"),
            HandRank::FullHouse => write!(f, "Full House"),
            HandRank::FourOfAKind => write!(f, "Four of a Kind"),
            HandRank::StraightFlush => write!(f, "Straight Flush"),
        }
    }
}

/// A fully evaluated 5-card poker hand, comparable to other hands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatedHand {
    pub rank: HandRank,
    /// Kickers for tiebreaking, ordered from most to least significant.
    /// For pairs: [pair rank, kicker1, kicker2, kicker3]
    /// For two pair: [high pair, low pair, kicker]
    /// etc.
    pub kickers: Vec<u8>,
    /// The 5 cards making up this hand.
    pub cards: Vec<Card>,
}

impl PartialOrd for EvaluatedHand {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EvaluatedHand {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank
            .cmp(&other.rank)
            .then_with(|| self.kickers.cmp(&other.kickers))
    }
}

impl fmt::Display for EvaluatedHand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cards: Vec<String> = self.cards.iter().map(|c| c.to_string()).collect();
        write!(f, "{} ({})", self.rank, cards.join(" "))
    }
}

fn rank_value(rank: Rank) -> u8 {
    match rank {
        Rank::Two => 2,
        Rank::Three => 3,
        Rank::Four => 4,
        Rank::Five => 5,
        Rank::Six => 6,
        Rank::Seven => 7,
        Rank::Eight => 8,
        Rank::Nine => 9,
        Rank::Ten => 10,
        Rank::Jack => 11,
        Rank::Queen => 12,
        Rank::King => 13,
        Rank::Ace => 14,
    }
}

/// Evaluate a 5-card hand.
fn evaluate_5(cards: &[Card; 5]) -> EvaluatedHand {
    let mut values: Vec<u8> = cards.iter().map(|c| rank_value(c.rank)).collect();
    values.sort_unstable_by(|a, b| b.cmp(a)); // Descending

    let is_flush = cards.iter().all(|c| c.suit == cards[0].suit);

    let is_straight = is_straight_values(&values);
    // Special case: A-2-3-4-5 (wheel)
    let is_wheel = values == vec![14, 5, 4, 3, 2];

    if is_flush && (is_straight || is_wheel) {
        let kickers = if is_wheel {
            vec![5] // 5-high straight flush
        } else {
            vec![values[0]]
        };
        return EvaluatedHand {
            rank: HandRank::StraightFlush,
            kickers,
            cards: cards.to_vec(),
        };
    }

    // Count rank occurrences
    let mut counts: Vec<(u8, u8)> = Vec::new(); // (count, value)
    let mut i = 0;
    while i < values.len() {
        let v = values[i];
        let mut count: usize = 1;
        while i + count < values.len() && values[i + count] == v {
            count += 1;
        }
        counts.push((count as u8, v));
        i += count;
    }
    // Sort by count descending, then value descending
    counts.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));

    let pattern: Vec<u8> = counts.iter().map(|(c, _)| *c).collect();

    match pattern.as_slice() {
        [4, 1] => EvaluatedHand {
            rank: HandRank::FourOfAKind,
            kickers: counts.iter().map(|(_, v)| *v).collect(),
            cards: cards.to_vec(),
        },
        [3, 2] => EvaluatedHand {
            rank: HandRank::FullHouse,
            kickers: counts.iter().map(|(_, v)| *v).collect(),
            cards: cards.to_vec(),
        },
        _ if is_flush => EvaluatedHand {
            rank: HandRank::Flush,
            kickers: values,
            cards: cards.to_vec(),
        },
        _ if is_straight || is_wheel => {
            let kickers = if is_wheel { vec![5] } else { vec![values[0]] };
            EvaluatedHand {
                rank: HandRank::Straight,
                kickers,
                cards: cards.to_vec(),
            }
        }
        [3, 1, 1] => EvaluatedHand {
            rank: HandRank::ThreeOfAKind,
            kickers: counts.iter().map(|(_, v)| *v).collect(),
            cards: cards.to_vec(),
        },
        [2, 2, 1] => EvaluatedHand {
            rank: HandRank::TwoPair,
            kickers: counts.iter().map(|(_, v)| *v).collect(),
            cards: cards.to_vec(),
        },
        [2, 1, 1, 1] => EvaluatedHand {
            rank: HandRank::OnePair,
            kickers: counts.iter().map(|(_, v)| *v).collect(),
            cards: cards.to_vec(),
        },
        _ => EvaluatedHand {
            rank: HandRank::HighCard,
            kickers: values,
            cards: cards.to_vec(),
        },
    }
}

fn is_straight_values(values: &[u8]) -> bool {
    if values.len() != 5 {
        return false;
    }
    // Values are sorted descending
    for i in 0..4 {
        if values[i] != values[i + 1] + 1 {
            return false;
        }
    }
    true
}

/// Evaluate the best 5-card hand from 7 cards.
pub fn best_hand(cards: &[Card]) -> EvaluatedHand {
    assert!(cards.len() >= 5);
    let mut best: Option<EvaluatedHand> = None;

    // Enumerate all C(n, 5) combinations
    let n = cards.len();
    for a in 0..n {
        for b in (a + 1)..n {
            for c in (b + 1)..n {
                for d in (c + 1)..n {
                    for e in (d + 1)..n {
                        let five = [cards[a], cards[b], cards[c], cards[d], cards[e]];
                        let hand = evaluate_5(&five);
                        best = Some(match best {
                            None => hand,
                            Some(prev) => {
                                if hand > prev {
                                    hand
                                } else {
                                    prev
                                }
                            }
                        });
                    }
                }
            }
        }
    }

    best.unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Card, Rank, Suit};

    fn c(rank: Rank, suit: Suit) -> Card {
        Card::new(rank, suit)
    }

    #[test]
    fn royal_flush() {
        let cards = vec![
            c(Rank::Ace, Suit::Spades),
            c(Rank::King, Suit::Spades),
            c(Rank::Queen, Suit::Spades),
            c(Rank::Jack, Suit::Spades),
            c(Rank::Ten, Suit::Spades),
            c(Rank::Two, Suit::Hearts),
            c(Rank::Three, Suit::Diamonds),
        ];
        let hand = best_hand(&cards);
        assert_eq!(hand.rank, HandRank::StraightFlush);
        assert_eq!(hand.kickers, vec![14]);
    }

    #[test]
    fn wheel_straight() {
        let cards = vec![
            c(Rank::Ace, Suit::Spades),
            c(Rank::Two, Suit::Hearts),
            c(Rank::Three, Suit::Diamonds),
            c(Rank::Four, Suit::Clubs),
            c(Rank::Five, Suit::Spades),
            c(Rank::King, Suit::Hearts),
            c(Rank::Queen, Suit::Diamonds),
        ];
        let hand = best_hand(&cards);
        assert_eq!(hand.rank, HandRank::Straight);
        assert_eq!(hand.kickers, vec![5]); // 5-high
    }

    #[test]
    fn full_house() {
        let cards = vec![
            c(Rank::King, Suit::Spades),
            c(Rank::King, Suit::Hearts),
            c(Rank::King, Suit::Diamonds),
            c(Rank::Jack, Suit::Clubs),
            c(Rank::Jack, Suit::Spades),
            c(Rank::Two, Suit::Hearts),
            c(Rank::Three, Suit::Diamonds),
        ];
        let hand = best_hand(&cards);
        assert_eq!(hand.rank, HandRank::FullHouse);
        assert_eq!(hand.kickers, vec![13, 11]); // Kings full of Jacks
    }

    #[test]
    fn two_pair() {
        let cards = vec![
            c(Rank::Ace, Suit::Spades),
            c(Rank::Ace, Suit::Hearts),
            c(Rank::King, Suit::Diamonds),
            c(Rank::King, Suit::Clubs),
            c(Rank::Queen, Suit::Spades),
            c(Rank::Two, Suit::Hearts),
            c(Rank::Three, Suit::Diamonds),
        ];
        let hand = best_hand(&cards);
        assert_eq!(hand.rank, HandRank::TwoPair);
        assert_eq!(hand.kickers, vec![14, 13, 12]); // Aces and Kings, Q kicker
    }

    #[test]
    fn flush_beats_straight() {
        let flush_cards = vec![
            c(Rank::Two, Suit::Spades),
            c(Rank::Five, Suit::Spades),
            c(Rank::Seven, Suit::Spades),
            c(Rank::Nine, Suit::Spades),
            c(Rank::Jack, Suit::Spades),
            c(Rank::Three, Suit::Hearts),
            c(Rank::Four, Suit::Diamonds),
        ];
        let straight_cards = vec![
            c(Rank::Six, Suit::Spades),
            c(Rank::Seven, Suit::Hearts),
            c(Rank::Eight, Suit::Diamonds),
            c(Rank::Nine, Suit::Clubs),
            c(Rank::Ten, Suit::Spades),
            c(Rank::Two, Suit::Hearts),
            c(Rank::Three, Suit::Diamonds),
        ];
        let flush = best_hand(&flush_cards);
        let straight = best_hand(&straight_cards);
        assert!(flush > straight);
    }

    #[test]
    fn four_of_a_kind() {
        let cards = vec![
            c(Rank::Ace, Suit::Spades),
            c(Rank::Ace, Suit::Hearts),
            c(Rank::Ace, Suit::Diamonds),
            c(Rank::Ace, Suit::Clubs),
            c(Rank::King, Suit::Spades),
            c(Rank::Two, Suit::Hearts),
            c(Rank::Three, Suit::Diamonds),
        ];
        let hand = best_hand(&cards);
        assert_eq!(hand.rank, HandRank::FourOfAKind);
    }

    #[test]
    fn high_card() {
        let cards = vec![
            c(Rank::Ace, Suit::Spades),
            c(Rank::King, Suit::Hearts),
            c(Rank::Queen, Suit::Diamonds),
            c(Rank::Jack, Suit::Clubs),
            c(Rank::Nine, Suit::Spades),
            c(Rank::Two, Suit::Hearts),
            c(Rank::Three, Suit::Diamonds),
        ];
        let hand = best_hand(&cards);
        assert_eq!(hand.rank, HandRank::HighCard);
        assert_eq!(hand.kickers, vec![14, 13, 12, 11, 9]);
    }
}
