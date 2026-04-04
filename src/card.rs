use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Suit {
    Clubs,
    Diamonds,
    Hearts,
    Spades,
}

impl Suit {
    pub const ALL: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];
}

impl fmt::Display for Suit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Suit::Clubs => write!(f, "c"),
            Suit::Diamonds => write!(f, "d"),
            Suit::Hearts => write!(f, "h"),
            Suit::Spades => write!(f, "s"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Rank {
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
    Ace,
}

impl Rank {
    pub const ALL: [Rank; 13] = [
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ten,
        Rank::Jack,
        Rank::Queen,
        Rank::King,
        Rank::Ace,
    ];
}

impl fmt::Display for Rank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rank::Two => write!(f, "2"),
            Rank::Three => write!(f, "3"),
            Rank::Four => write!(f, "4"),
            Rank::Five => write!(f, "5"),
            Rank::Six => write!(f, "6"),
            Rank::Seven => write!(f, "7"),
            Rank::Eight => write!(f, "8"),
            Rank::Nine => write!(f, "9"),
            Rank::Ten => write!(f, "T"),
            Rank::Jack => write!(f, "J"),
            Rank::Queen => write!(f, "Q"),
            Rank::King => write!(f, "K"),
            Rank::Ace => write!(f, "A"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Card {
    pub rank: Rank,
    pub suit: Suit,
}

impl Card {
    pub fn new(rank: Rank, suit: Suit) -> Self {
        Self { rank, suit }
    }

    /// Returns the card's index in a standard 52-card deck (0..51).
    pub fn index(&self) -> u8 {
        let suit_idx = match self.suit {
            Suit::Clubs => 0,
            Suit::Diamonds => 1,
            Suit::Hearts => 2,
            Suit::Spades => 3,
        };
        let rank_idx = match self.rank {
            Rank::Two => 0,
            Rank::Three => 1,
            Rank::Four => 2,
            Rank::Five => 3,
            Rank::Six => 4,
            Rank::Seven => 5,
            Rank::Eight => 6,
            Rank::Nine => 7,
            Rank::Ten => 8,
            Rank::Jack => 9,
            Rank::Queen => 10,
            Rank::King => 11,
            Rank::Ace => 12,
        };
        suit_idx * 13 + rank_idx
    }

    /// Create a card from its deck index (0..51).
    pub fn from_index(index: u8) -> crate::Result<Self> {
        if index >= 52 {
            return Err(crate::Error::InvalidCard(index));
        }
        let suit = Suit::ALL[(index / 13) as usize];
        let rank = Rank::ALL[(index % 13) as usize];
        Ok(Card { rank, suit })
    }

    /// Returns all 52 cards in canonical order.
    pub fn deck() -> Vec<Card> {
        let mut cards = Vec::with_capacity(52);
        for suit in Suit::ALL {
            for rank in Rank::ALL {
                cards.push(Card { rank, suit });
            }
        }
        cards
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.rank, self.suit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_has_52_cards() {
        assert_eq!(Card::deck().len(), 52);
    }

    #[test]
    fn index_roundtrip() {
        for card in Card::deck() {
            let idx = card.index();
            let back = Card::from_index(idx).unwrap();
            assert_eq!(card, back);
        }
    }

    #[test]
    fn display() {
        let card = Card::new(Rank::Ace, Suit::Spades);
        assert_eq!(card.to_string(), "As");
    }
}
