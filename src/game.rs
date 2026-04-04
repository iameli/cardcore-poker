//! Hold'em game state and betting logic.

use serde::{Deserialize, Serialize};

use crate::crypto::Point;

/// Identifies a player by their index in the game (0-based, in seat order).
pub type PlayerId = usize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
    Showdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BetAction {
    Fold,
    Check,
    Call,
    Raise(u64),
    AllIn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub chips: u64,
    pub bet_this_street: u64,
    pub folded: bool,
    pub all_in: bool,
    /// Encrypted hole cards (2 points, only known to this player until showdown).
    pub hole_encrypted: Vec<Point>,
    /// Decrypted hole cards (filled in once all other players provide decryption shares).
    pub hole_points: Vec<Point>,
}

impl PlayerState {
    pub fn new(chips: u64) -> Self {
        Self {
            chips,
            bet_this_street: 0,
            folded: false,
            all_in: false,
            hole_encrypted: Vec::new(),
            hole_points: Vec::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        !self.folded && !self.all_in
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub players: Vec<PlayerState>,
    pub street: Street,
    pub pot: u64,
    pub current_bet: u64,
    /// Index of the dealer button.
    pub button: usize,
    /// Whose turn it is to act (for betting).
    pub action_on: Option<PlayerId>,
    /// The encrypted deck after all players have shuffled.
    pub deck: Vec<Point>,
    /// Community card points (revealed progressively).
    pub community: Vec<Point>,
    /// Small blind amount.
    pub small_blind: u64,
    /// Big blind amount.
    pub big_blind: u64,
    /// Number of players who have acted this betting round.
    pub actions_this_round: usize,
}

impl GameState {
    pub fn new(num_players: usize, starting_chips: u64, small_blind: u64) -> Self {
        assert!(num_players >= 2 && num_players <= 10);
        Self {
            players: (0..num_players)
                .map(|_| PlayerState::new(starting_chips))
                .collect(),
            street: Street::Preflop,
            pot: 0,
            current_bet: 0,
            button: 0,
            action_on: None,
            deck: Vec::new(),
            community: Vec::new(),
            small_blind,
            big_blind: small_blind * 2,
            actions_this_round: 0,
        }
    }

    pub fn num_players(&self) -> usize {
        self.players.len()
    }

    /// Players still in the hand (not folded).
    pub fn active_player_count(&self) -> usize {
        self.players.iter().filter(|p| !p.folded).count()
    }

    /// Players who can still act (not folded, not all-in).
    pub fn actionable_player_count(&self) -> usize {
        self.players.iter().filter(|p| p.is_active()).count()
    }

    /// Next player seat after `seat` who is still active (not folded, not all-in).
    pub fn next_active_seat(&self, seat: usize) -> Option<usize> {
        let n = self.num_players();
        for i in 1..n {
            let idx = (seat + i) % n;
            if self.players[idx].is_active() {
                return Some(idx);
            }
        }
        None
    }

    /// Next player seat after `seat` who hasn't folded (may be all-in).
    pub fn next_not_folded_seat(&self, seat: usize) -> Option<usize> {
        let n = self.num_players();
        for i in 1..n {
            let idx = (seat + i) % n;
            if !self.players[idx].folded {
                return Some(idx);
            }
        }
        None
    }

    /// The small blind seat (left of button).
    pub fn small_blind_seat(&self) -> usize {
        let n = self.num_players();
        if n == 2 {
            // Heads-up: button is SB
            self.button
        } else {
            (self.button + 1) % n
        }
    }

    /// The big blind seat.
    pub fn big_blind_seat(&self) -> usize {
        let n = self.num_players();
        if n == 2 {
            (self.button + 1) % n
        } else {
            (self.button + 2) % n
        }
    }

    /// Post blinds, deducting from player chips and adding to pot.
    pub fn post_blinds(&mut self) {
        let sb_seat = self.small_blind_seat();
        let bb_seat = self.big_blind_seat();

        let sb_amount = self.small_blind.min(self.players[sb_seat].chips);
        self.players[sb_seat].chips -= sb_amount;
        self.players[sb_seat].bet_this_street = sb_amount;
        self.pot += sb_amount;

        let bb_amount = self.big_blind.min(self.players[bb_seat].chips);
        self.players[bb_seat].chips -= bb_amount;
        self.players[bb_seat].bet_this_street = bb_amount;
        self.pot += bb_amount;

        self.current_bet = bb_amount;
    }

    /// Set action_on to the first player who should act for this street.
    pub fn start_betting_round(&mut self) {
        self.actions_this_round = 0;
        let n = self.num_players();

        let first_seat = if self.street == Street::Preflop {
            // Preflop: action starts left of BB
            let bb = self.big_blind_seat();
            // Find next active player after BB
            let mut seat = (bb + 1) % n;
            loop {
                if self.players[seat].is_active() {
                    break seat;
                }
                seat = (seat + 1) % n;
                if seat == bb {
                    break bb; // Wrapped around
                }
            }
        } else {
            // Postflop: action starts left of button
            let mut seat = (self.button + 1) % n;
            loop {
                if self.players[seat].is_active() {
                    break seat;
                }
                seat = (seat + 1) % n;
                if seat == self.button {
                    break self.button;
                }
            }
        };

        self.action_on = if self.actionable_player_count() > 0 {
            Some(first_seat)
        } else {
            None
        };
    }

    /// Apply a bet action. Returns true if the betting round is over.
    pub fn apply_bet(&mut self, player_id: PlayerId, action: &BetAction) -> bool {
        let to_call = self.current_bet.saturating_sub(self.players[player_id].bet_this_street);

        match action {
            BetAction::Fold => {
                self.players[player_id].folded = true;
            }
            BetAction::Check => {
                // Nothing changes
            }
            BetAction::Call => {
                let amount = to_call.min(self.players[player_id].chips);
                self.players[player_id].chips -= amount;
                self.players[player_id].bet_this_street += amount;
                self.pot += amount;
            }
            BetAction::Raise(total) => {
                let amount = total.saturating_sub(self.players[player_id].bet_this_street);
                let amount = amount.min(self.players[player_id].chips);
                self.players[player_id].chips -= amount;
                self.players[player_id].bet_this_street += amount;
                self.pot += amount;
                self.current_bet = self.players[player_id].bet_this_street;
                // Reset action count — everyone needs to act again
                self.actions_this_round = 0;
            }
            BetAction::AllIn => {
                let amount = self.players[player_id].chips;
                self.players[player_id].chips = 0;
                self.players[player_id].bet_this_street += amount;
                self.players[player_id].all_in = true;
                self.pot += amount;
                if self.players[player_id].bet_this_street > self.current_bet {
                    self.current_bet = self.players[player_id].bet_this_street;
                    self.actions_this_round = 0;
                }
            }
        }

        self.actions_this_round += 1;

        // Check if betting round is over
        if self.active_player_count() <= 1 {
            // Everyone folded or only one left
            return true;
        }
        if self.actionable_player_count() == 0 {
            // Everyone is all-in or folded
            return true;
        }

        // Advance to next active player
        if let Some(next) = self.next_active_seat(player_id) {
            self.action_on = Some(next);
        }

        // Round is over when everyone has acted and bets are level
        let all_bets_level = self
            .players
            .iter()
            .all(|p| p.folded || p.all_in || p.bet_this_street == self.current_bet);

        self.actions_this_round >= self.actionable_player_count() && all_bets_level
    }

    /// Reset per-street betting state for a new street.
    pub fn new_street(&mut self, street: Street) {
        self.street = street;
        self.current_bet = 0;
        self.actions_this_round = 0;
        for p in &mut self.players {
            p.bet_this_street = 0;
        }
    }
}
