//! Integration test: drives a complete Hold'em hand through the protocol,
//! including a fuzz test that randomly picks valid actions.

use cardcore_poker::crypto::{self, PlayerKeys, Point};
use cardcore_poker::game::BetAction;
use cardcore_poker::protocol::{Action, Phase, ProtocolState, ValidActionKind};
use rand::prelude::*;
use rand::rngs::StdRng;
use std::collections::HashMap;

struct SimPlayer {
    id: usize,
    keys: PlayerKeys,
    seed: Vec<u8>,
}

impl SimPlayer {
    fn new(id: usize) -> Self {
        crypto::init().unwrap();
        let keys = PlayerKeys::generate().unwrap();
        let seed = format!("player_{}_seed_{}", id, rand::random::<u64>()).into_bytes();
        Self { id, keys, seed }
    }

    fn commitment(&self) -> [u8; crypto::HASH_BYTES] {
        crypto::blake2b(&self.seed).unwrap()
    }

    fn encrypt_and_shuffle(&self, deck: &[Point], rng: &mut impl Rng) -> Vec<Point> {
        let mut encrypted = self.keys.encrypt_deck(deck).unwrap();
        encrypted.shuffle(rng);
        encrypted
    }
}

/// Drive a complete 2-player hand through the protocol.
#[test]
fn full_hand_2_players() {
    let mut state = ProtocolState::new(2, 1000, 10);
    let players: Vec<SimPlayer> = (0..2).map(SimPlayer::new).collect();
    let mut rng = StdRng::seed_from_u64(42);

    // Build card lookup
    let card_map: HashMap<Point, cardcore_poker::card::Card> = crypto::card_points()
        .unwrap()
        .into_iter()
        .map(|(c, p)| (p, c))
        .collect();

    // Join
    for p in &players {
        state.apply(&Action::Join { player_id: p.id }).unwrap();
    }
    assert!(matches!(state.phase, Phase::CommitSeeds));

    // Commit + reveal seeds
    for p in &players {
        state
            .apply(&Action::CommitSeed {
                player_id: p.id,
                commitment: p.commitment(),
            })
            .unwrap();
    }
    for p in &players {
        state
            .apply(&Action::RevealSeed {
                player_id: p.id,
                seed: p.seed.clone(),
            })
            .unwrap();
    }

    // Shuffle
    for p in &players {
        let shuffled = p.encrypt_and_shuffle(&state.game.deck, &mut rng);
        state
            .apply(&Action::ShuffleDeck {
                player_id: p.id,
                deck: shuffled,
            })
            .unwrap();
    }
    assert!(matches!(state.phase, Phase::Dealing { .. }));
    assert_eq!(state.game.pot, 30);

    // Deal all hole cards
    deal_until_done(&mut state, &players);

    assert!(matches!(state.phase, Phase::Betting));
    for p in &state.game.players {
        assert_eq!(p.hole_encrypted.len(), 2);
    }

    // Verify that decryption produces valid cards
    for p in &players {
        for enc in &state.game.players[p.id].hole_encrypted {
            let decrypted = p.keys.decrypt_point(enc).unwrap();
            assert!(card_map.contains_key(&decrypted), "hole card should resolve to a known card");
        }
    }

    // Play through remaining streets
    play_betting_round(&mut state); // preflop
    deal_until_done(&mut state, &players); // flop
    assert_eq!(state.game.community.len(), 3);

    // Verify community cards resolve
    for cp in &state.game.community {
        assert!(card_map.contains_key(cp), "community card should resolve");
    }

    play_betting_round(&mut state); // flop betting
    deal_until_done(&mut state, &players); // turn
    assert_eq!(state.game.community.len(), 4);

    play_betting_round(&mut state); // turn betting
    deal_until_done(&mut state, &players); // river
    assert_eq!(state.game.community.len(), 5);

    play_betting_round(&mut state); // river betting

    assert!(matches!(state.phase, Phase::Showdown));

    // Showdown: reveal keys
    for p in &players {
        if !state.game.players[p.id].folded {
            state
                .apply(&Action::RevealHand {
                    player_id: p.id,
                    scalar: p.keys.decrypt.clone(),
                })
                .unwrap();
        }
    }
    assert!(matches!(state.phase, Phase::Complete));

    // Verify hole_points are real cards
    for p in &players {
        for pt in &state.game.players[p.id].hole_points {
            assert!(card_map.contains_key(pt), "revealed card should resolve");
        }
    }
}

/// Fuzz test: randomly pick valid actions until the game completes.
#[test]
fn fuzz_random_actions() {
    for seed in 0..20 {
        let mut rng = StdRng::seed_from_u64(seed);
        let num_players = rng.random_range(2..=4);
        let mut state = ProtocolState::new(num_players, 1000, 10);
        let players: Vec<SimPlayer> = (0..num_players).map(SimPlayer::new).collect();

        let mut steps = 0;
        let max_steps = 10000;

        while !matches!(state.phase, Phase::Complete) {
            let actions = state.valid_actions();
            if actions.is_empty() {
                panic!(
                    "seed={}: stuck with no valid actions in phase {:?} after {} steps",
                    seed, state.phase, steps
                );
            }

            let va = &actions[rng.random_range(0..actions.len())];
            let action = make_action(va, &players, &state, &mut rng);
            state.apply(&action).unwrap_or_else(|e| {
                panic!(
                    "seed={}: action failed in phase {:?}: {}",
                    seed, state.phase, e
                );
            });

            steps += 1;
            if steps > max_steps {
                panic!(
                    "seed={}: exceeded {} steps in phase {:?}",
                    seed, max_steps, state.phase
                );
            }
        }
        eprintln!(
            "seed={}: completed in {} steps ({} players)",
            seed, steps, num_players
        );
    }
}

fn make_action(
    va: &cardcore_poker::protocol::ValidAction,
    players: &[SimPlayer],
    state: &ProtocolState,
    rng: &mut impl Rng,
) -> Action {
    match &va.kind {
        ValidActionKind::Join => Action::Join {
            player_id: va.player_id,
        },
        ValidActionKind::CommitSeed => Action::CommitSeed {
            player_id: va.player_id,
            commitment: players[va.player_id].commitment(),
        },
        ValidActionKind::RevealSeed => Action::RevealSeed {
            player_id: va.player_id,
            seed: players[va.player_id].seed.clone(),
        },
        ValidActionKind::ShuffleDeck => {
            let shuffled = players[va.player_id].encrypt_and_shuffle(&state.game.deck, rng);
            Action::ShuffleDeck {
                player_id: va.player_id,
                deck: shuffled,
            }
        }
        ValidActionKind::DecryptCard { deck_position } => {
            // Decrypt the current point from the dealing phase
            let current_point = match &state.phase {
                Phase::Dealing { current_point, .. } => current_point.clone(),
                _ => panic!("expected Dealing phase"),
            };
            let result = players[va.player_id]
                .keys
                .decrypt_point(&current_point)
                .unwrap();
            Action::DecryptCard {
                player_id: va.player_id,
                deck_position: *deck_position,
                result,
            }
        }
        ValidActionKind::Bet { options } => {
            let action = options[rng.random_range(0..options.len())].clone();
            Action::Bet {
                player_id: va.player_id,
                action,
            }
        }
        ValidActionKind::RevealHand => Action::RevealHand {
            player_id: va.player_id,
            scalar: players[va.player_id].keys.decrypt.clone(),
        },
    }
}

/// Process all dealing actions until the phase changes.
fn deal_until_done(state: &mut ProtocolState, players: &[SimPlayer]) {
    while matches!(state.phase, Phase::Dealing { .. }) {
        let actions = state.valid_actions();
        if actions.is_empty() {
            break;
        }
        let va = &actions[0];
        if let ValidActionKind::DecryptCard { deck_position } = &va.kind {
            let current_point = match &state.phase {
                Phase::Dealing { current_point, .. } => current_point.clone(),
                _ => break,
            };
            let result = players[va.player_id]
                .keys
                .decrypt_point(&current_point)
                .unwrap();
            state
                .apply(&Action::DecryptCard {
                    player_id: va.player_id,
                    deck_position: *deck_position,
                    result,
                })
                .unwrap();
        }
    }
}

fn play_betting_round(state: &mut ProtocolState) {
    while matches!(state.phase, Phase::Betting) {
        let actions = state.valid_actions();
        if actions.is_empty() {
            break;
        }
        if let ValidActionKind::Bet { options } = &actions[0].kind {
            let action = if options.iter().any(|a| matches!(a, BetAction::Check)) {
                BetAction::Check
            } else {
                BetAction::Call
            };
            state
                .apply(&Action::Bet {
                    player_id: actions[0].player_id,
                    action,
                })
                .unwrap();
        }
    }
}
