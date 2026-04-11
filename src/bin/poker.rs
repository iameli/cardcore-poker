//! Text-based poker CLI.
//!
//! All actions are logged as newline-delimited JSON to `game.ndjson`.
//! After the hand, seeds are revealed — anyone can replay the log and verify.

use cardcore_poker::card::Card;
use cardcore_poker::crypto::{self, PlayerKeys, PlayerRng, Point};
use cardcore_poker::eval;
use cardcore_poker::game::BetAction;
use cardcore_poker::protocol::{Action, Phase, ProtocolState, ValidActionKind};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};

/// Wraps ProtocolState and logs every action.
struct Game {
    state: ProtocolState,
    log: Vec<Action>,
}

impl Game {
    fn new(num_players: usize, starting_chips: u64, small_blind: u64) -> Self {
        Self {
            state: ProtocolState::new(num_players, starting_chips, small_blind),
            log: Vec::new(),
        }
    }

    fn apply(&mut self, action: &Action) {
        self.state.apply(action).unwrap();
        self.log.push(action.clone());
    }

    fn write_log(&self, path: &str) {
        let mut out = String::new();
        for action in &self.log {
            out.push_str(&serde_json::to_string(action).unwrap());
            out.push('\n');
        }
        fs::write(path, &out).unwrap();
    }
}

struct Player {
    id: usize,
    name: String,
    seed: Vec<u8>,
    keys: PlayerKeys,
}

impl Player {
    fn new(id: usize, name: String) -> Self {
        let seed = format!("seed_{}_{}", id, rand::random::<u64>()).into_bytes();
        let mut rng = PlayerRng::new(&seed, b"shuffle").unwrap();
        let keys = PlayerKeys::generate(&mut rng).unwrap();
        Self { id, name, seed, keys }
    }

    fn commitment(&self) -> [u8; crypto::HASH_BYTES] {
        crypto::blake2b(&self.seed).unwrap()
    }

    fn encrypt_and_shuffle(&self, deck: &[Point]) -> Vec<Point> {
        let mut encrypted = self.keys.encrypt_deck(deck).unwrap();
        let mut rng = PlayerRng::new(&self.seed, b"shuffle_permutation").unwrap();
        encrypted.shuffle(rng.as_rng());
        encrypted
    }

    fn lock_deck(&mut self, deck: &[Point]) -> Vec<Point> {
        let deck_hash = crypto::blake2b(&serde_json::to_vec(deck).unwrap()).unwrap();
        let mut context = b"lock:".to_vec();
        context.extend_from_slice(&deck_hash);
        let mut rng = PlayerRng::new(&self.seed, &context).unwrap();
        self.keys.generate_lock_keys(deck.len(), &mut rng).unwrap();
        self.keys.lock_deck(deck).unwrap()
    }
}

use rand::prelude::SliceRandom;

fn point_to_card(point: &Point, card_map: &HashMap<Point, Card>) -> Option<Card> {
    card_map.get(point).copied()
}

fn read_line(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

fn clear_screen() {
    print!("\x1b[2J\x1b[H");
    io::stdout().flush().unwrap();
}

fn pause(msg: &str) {
    read_line(&format!("{} [press enter]", msg));
}

fn main() {
    crypto::init().unwrap();

    println!("=== CARDCORE POKER ===");
    println!("Texas Hold'em with mental poker cryptography\n");

    let num_players: usize = loop {
        let input = read_line("Number of players (2-6): ");
        match input.parse() {
            Ok(n) if (2..=6).contains(&n) => break n,
            _ => println!("Please enter a number between 2 and 6."),
        }
    };

    let mut player_names = Vec::new();
    for i in 0..num_players {
        let name = read_line(&format!("Player {} name: ", i + 1));
        let name = if name.is_empty() {
            format!("Player {}", i + 1)
        } else {
            name
        };
        player_names.push(name);
    }

    let mut players: Vec<Player> = player_names
        .iter()
        .enumerate()
        .map(|(i, name)| Player::new(i, name.clone()))
        .collect();

    let card_map: HashMap<Point, Card> = crypto::card_points()
        .unwrap()
        .into_iter()
        .map(|(c, p)| (p, c))
        .collect();

    let mut game = Game::new(num_players, 1000, 10);

    println!("\n--- Setting up hand ---\n");

    // Join
    for p in &players {
        game.apply(&Action::Join { player_id: p.id });
    }
    println!("All {} players joined.", num_players);

    // Commit seeds
    for p in &players {
        game.apply(&Action::CommitSeed {
            player_id: p.id,
            commitment: p.commitment(),
        });
    }
    println!("Seeds committed (kept secret until end of hand).");

    // Shuffle
    for p in &players {
        let shuffled = p.encrypt_and_shuffle(&game.state.game.deck);
        game.apply(&Action::ShuffleDeck {
            player_id: p.id,
            deck: shuffled,
        });
        println!("{} shuffled the deck.", p.name);
    }

    // Lock
    for i in 0..players.len() {
        let locked = players[i].lock_deck(&game.state.game.deck);
        game.apply(&Action::LockDeck {
            player_id: i,
            deck: locked,
        });
        println!("{} locked the deck.", players[i].name);
    }

    println!(
        "\nBlinds: {}/{} (SB/BB)",
        game.state.game.small_blind, game.state.game.big_blind
    );
    println!("{} is the dealer.\n", players[game.state.game.button].name);

    // Deal hole cards
    println!("--- Dealing hole cards ---\n");
    deal_phase(&mut game, &players);

    // Resolve hole cards
    let mut player_hole_cards: Vec<Vec<Card>> = Vec::new();
    for p in &players {
        let mut cards = Vec::new();
        for (idx, encrypted_point) in game.state.game.players[p.id].hole_encrypted.iter().enumerate() {
            let pos = game.state.hole_card_positions[p.id][idx];
            let decrypted = crypto::decrypt(encrypted_point, &p.keys.lock_decrypt[pos]).unwrap();
            if let Some(card) = point_to_card(&decrypted, &card_map) {
                cards.push(card);
            }
        }
        player_hole_cards.push(cards);
    }

    // Show cards (hot-seat)
    for p in &players {
        clear_screen();
        pause(&format!("Pass the terminal to {}.", p.name));
        println!(
            "\n{}'s hole cards: {}",
            p.name,
            player_hole_cards[p.id].iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
        );
        println!("Chips: {}\n", game.state.game.players[p.id].chips);
        pause("Memorize your cards, then press enter to hide them.");
    }
    clear_screen();

    // Main game loop
    loop {
        match &game.state.phase {
            Phase::Betting => {
                print_table_state(&game.state, &players, &card_map);
                let actions = game.state.valid_actions();
                if actions.is_empty() {
                    break;
                }
                let va = &actions[0];
                if let ValidActionKind::Bet { options } = &va.kind {
                    let bet = prompt_bet(&players[va.player_id], options, &game.state);
                    game.apply(&Action::Bet {
                        player_id: va.player_id,
                        action: bet,
                    });
                }
            }
            Phase::Dealing { .. } => {
                let street_name = match game.state.game.community.len() {
                    0 => "Flop",
                    3 => "Turn",
                    4 => "River",
                    _ => "Community",
                };
                println!("\n--- Dealing {} ---\n", street_name);
                deal_phase(&mut game, &players);

                let community_cards = resolve_community(&game.state, &card_map);
                println!(
                    "Community: {}\n",
                    community_cards.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
                );
            }
            Phase::Showdown => {
                println!("\n--- Showdown ---\n");
                let community_cards = resolve_community(&game.state, &card_map);

                for p in &players {
                    if game.state.game.players[p.id].folded {
                        continue;
                    }
                    let scalars: Vec<(usize, _)> = game.state.hole_card_positions[p.id]
                        .iter()
                        .map(|pos| (*pos, p.keys.lock_decrypt[*pos].clone()))
                        .collect();
                    game.apply(&Action::RevealHand {
                        player_id: p.id,
                        scalars,
                    });
                }

                println!(
                    "Community: {}\n",
                    community_cards.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
                );

                let mut results: Vec<(usize, eval::EvaluatedHand)> = Vec::new();
                for p in &players {
                    if game.state.game.players[p.id].folded {
                        println!("{}: folded", p.name);
                        continue;
                    }
                    let hole = &player_hole_cards[p.id];
                    println!(
                        "{}: {}",
                        p.name,
                        hole.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
                    );

                    let mut all_cards = hole.clone();
                    all_cards.extend_from_slice(&community_cards);
                    if all_cards.len() >= 5 {
                        let hand = eval::best_hand(&all_cards);
                        println!("  -> {}", hand);
                        results.push((p.id, hand));
                    }
                }

                if let Some(best) = results.iter().map(|(_, h)| h).max() {
                    let winners: Vec<_> = results.iter().filter(|(_, h)| h == best).collect();
                    let pot = game.state.game.pot;
                    let share = pot / winners.len() as u64;
                    println!();
                    if winners.len() == 1 {
                        println!("{} wins {} chips with {}!", players[winners[0].0].name, pot, best);
                    } else {
                        let names: Vec<_> = winners.iter().map(|(id, _)| players[*id].name.as_str()).collect();
                        println!("Split pot! {} each win {} chips with {}.", names.join(" and "), share, best);
                    }
                }
                break;
            }
            Phase::Complete => {
                if game.state.game.active_player_count() == 1 {
                    let winner = game.state.game.players.iter().enumerate().find(|(_, p)| !p.folded).unwrap().0;
                    println!("\n{} wins {} chips (everyone else folded)!", players[winner].name, game.state.game.pot);
                }
                break;
            }
            _ => break,
        }
    }

    // Post-game: verify seeds
    println!("\n--- Seed Verification ---\n");
    for p in &players {
        game.apply(&Action::VerifySeed {
            player_id: p.id,
            seed: p.seed.clone(),
        });
        println!("{}'s seed verified.", p.name);
    }
    println!("All seeds verified. Game is fully replayable.");

    // Write game log
    let log_path = "game.ndjson";
    game.write_log(log_path);
    println!("\nGame log written to {}", log_path);

    println!("\n--- Hand complete ---");
    println!("\nFinal chip counts:");
    for p in &players {
        println!("  {}: {} chips", p.name, game.state.game.players[p.id].chips);
    }
}

fn deal_phase(game: &mut Game, players: &[Player]) {
    while matches!(game.state.phase, Phase::Dealing { .. }) {
        let actions = game.state.valid_actions();
        if actions.is_empty() {
            break;
        }
        for va in &actions {
            if let ValidActionKind::RevealLockKey { deck_position } = &va.kind {
                game.apply(&Action::RevealLockKey {
                    player_id: va.player_id,
                    deck_position: *deck_position,
                    scalar: players[va.player_id].keys.lock_decrypt[*deck_position].clone(),
                });
            }
        }
    }
}

fn resolve_community(state: &ProtocolState, card_map: &HashMap<Point, Card>) -> Vec<Card> {
    state.game.community.iter().filter_map(|p| point_to_card(p, card_map)).collect()
}

fn print_table_state(state: &ProtocolState, players: &[Player], card_map: &HashMap<Point, Card>) {
    let street = match state.game.street {
        cardcore_poker::game::Street::Preflop => "PREFLOP",
        cardcore_poker::game::Street::Flop => "FLOP",
        cardcore_poker::game::Street::Turn => "TURN",
        cardcore_poker::game::Street::River => "RIVER",
        cardcore_poker::game::Street::Showdown => "SHOWDOWN",
    };

    let community = resolve_community(state, card_map);
    let community_str = if community.is_empty() {
        String::new()
    } else {
        format!("  Board: {}", community.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" "))
    };

    println!("\n[{}] Pot: {}{}", street, state.game.pot, community_str);
    for (i, ps) in state.game.players.iter().enumerate() {
        let status = if ps.folded { " (folded)" } else if ps.all_in { " (all-in)" } else { "" };
        let marker = if state.game.action_on == Some(i) { " <--" } else { "" };
        println!("  {}: {} chips (bet: {}){}{}", players[i].name, ps.chips, ps.bet_this_street, status, marker);
    }
    println!();
}

fn prompt_bet(player: &Player, options: &[BetAction], state: &ProtocolState) -> BetAction {
    loop {
        println!("{}'s turn. Options:", player.name);
        for (i, opt) in options.iter().enumerate() {
            let desc = match opt {
                BetAction::Fold => "Fold".to_string(),
                BetAction::Check => "Check".to_string(),
                BetAction::Call => {
                    let to_call = state.game.current_bet.saturating_sub(state.game.players[player.id].bet_this_street);
                    format!("Call ({})", to_call)
                }
                BetAction::Raise(amount) => format!("Raise to {}", amount),
                BetAction::AllIn => format!("All-in ({})", state.game.players[player.id].chips),
            };
            println!("  {}: {}", i + 1, desc);
        }

        let input = read_line("> ");

        if input.starts_with('r') || input.starts_with('R') {
            if let Ok(amount) = input[1..].trim().parse::<u64>() {
                let min_raise = state.game.big_blind;
                let to_call = state.game.current_bet.saturating_sub(state.game.players[player.id].bet_this_street);
                if amount > to_call + min_raise && amount <= state.game.players[player.id].chips {
                    return BetAction::Raise(amount);
                }
                println!("Invalid raise. Min: {}, Max: {}", to_call + min_raise, state.game.players[player.id].chips);
                continue;
            }
        }

        match input.parse::<usize>() {
            Ok(n) if n >= 1 && n <= options.len() => return options[n - 1].clone(),
            _ => println!("Enter a number 1-{}, or 'r<amount>' to raise.", options.len()),
        }
    }
}
