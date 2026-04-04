//! Integration test: drives a complete 2-player Hold'em hand through the protocol,
//! including a fuzz test that randomly picks valid actions.

use cardcore_poker::crypto::{self, PlayerKeys, Point};
use cardcore_poker::game::BetAction;
use cardcore_poker::protocol::{Action, Phase, ProtocolState, ValidActionKind};
use rand::prelude::*;
use rand::rngs::StdRng;

/// Simulates one player's view: they know their own keys but not others'.
struct SimPlayer {
    id: usize,
    keys: PlayerKeys,
    seed: Vec<u8>,
}

impl SimPlayer {
    fn new(id: usize) -> Self {
        crypto::init().unwrap();
        let keys = PlayerKeys::generate(52).unwrap();
        let seed = format!("player_{}_seed_{}", id, rand::random::<u64>())
            .into_bytes();
        Self { id, keys, seed }
    }

    fn commitment(&self) -> [u8; crypto::HASH_BYTES] {
        crypto::blake2b(&self.seed).unwrap()
    }

    /// Encrypt all cards in the deck with this player's per-position keys, then shuffle.
    fn encrypt_and_shuffle(&self, deck: &[Point], rng: &mut impl Rng) -> Vec<Point> {
        let mut encrypted: Vec<Point> = deck
            .iter()
            .enumerate()
            .map(|(i, p)| crypto::encrypt(p, &self.keys.encrypt[i]).unwrap())
            .collect();
        encrypted.shuffle(rng);
        encrypted
    }
}

/// Drive a complete 2-player hand through the protocol deterministically.
#[test]
fn full_hand_2_players() {
    let mut state = ProtocolState::new(2, 1000, 10);
    let players: Vec<SimPlayer> = (0..2).map(SimPlayer::new).collect();
    let mut rng = StdRng::seed_from_u64(42);

    // Join
    state.apply(&Action::Join { player_id: 0 }).unwrap();
    state.apply(&Action::Join { player_id: 1 }).unwrap();
    assert!(matches!(state.phase, Phase::CommitSeeds));

    // Commit seeds
    for p in &players {
        state
            .apply(&Action::CommitSeed {
                player_id: p.id,
                commitment: p.commitment(),
            })
            .unwrap();
    }
    assert!(matches!(state.phase, Phase::RevealSeeds));

    // Reveal seeds
    for p in &players {
        state
            .apply(&Action::RevealSeed {
                player_id: p.id,
                seed: p.seed.clone(),
            })
            .unwrap();
    }
    assert!(matches!(state.phase, Phase::Shuffle { .. }));

    // Shuffle — each player encrypts and shuffles
    for p in &players {
        let shuffled = p.encrypt_and_shuffle(&state.game.deck, &mut rng);
        state
            .apply(&Action::ShuffleDeck {
                player_id: p.id,
                deck: shuffled,
            })
            .unwrap();
    }
    // After shuffle, blinds are posted and we're dealing hole cards
    assert!(matches!(state.phase, Phase::DealHole { .. }));
    assert_eq!(state.game.pot, 30); // SB=10 + BB=20

    // Deal hole cards — for each card, all other players provide decrypt shares
    while matches!(state.phase, Phase::DealHole { .. }) {
        let actions = state.valid_actions();
        assert!(!actions.is_empty(), "no valid actions during deal");
        for va in &actions {
            if let ValidActionKind::DecryptCard { position } = &va.kind {
                state
                    .apply(&Action::DecryptCard {
                        player_id: va.player_id,
                        position: *position,
                        scalar: players[va.player_id].keys.decrypt[*position].clone(),
                    })
                    .unwrap();
            }
        }
    }
    assert!(matches!(state.phase, Phase::Betting));
    // Each player should have 2 hole cards (partially decrypted — missing their own key)
    for p in &state.game.players {
        assert_eq!(p.hole_encrypted.len(), 2);
    }

    // Preflop betting: just check/call through
    play_betting_round(&mut state);

    // Should be dealing flop now
    assert!(
        matches!(state.phase, Phase::DealCommunity { num_to_deal: 3 }),
        "expected DealCommunity for flop, got {:?}",
        state.phase
    );
    deal_community_cards(&mut state, &players);
    assert_eq!(state.game.community.len(), 3);

    // Flop betting
    play_betting_round(&mut state);

    // Turn
    assert!(matches!(state.phase, Phase::DealCommunity { num_to_deal: 1 }));
    deal_community_cards(&mut state, &players);
    assert_eq!(state.game.community.len(), 4);

    // Turn betting
    play_betting_round(&mut state);

    // River
    assert!(matches!(state.phase, Phase::DealCommunity { num_to_deal: 1 }));
    deal_community_cards(&mut state, &players);
    assert_eq!(state.game.community.len(), 5);

    // River betting
    play_betting_round(&mut state);

    // Showdown
    assert!(
        matches!(state.phase, Phase::Showdown),
        "expected Showdown, got {:?}",
        state.phase
    );

    // Each non-folded player reveals their hole card decryption keys
    for p in &players {
        if !state.game.players[p.id].folded {
            let hole_positions: Vec<usize> = (0..2)
                .map(|card_idx| state.deck_position_for_hole_public(p.id, card_idx))
                .collect();
            let scalars: Vec<_> = hole_positions
                .iter()
                .map(|pos| players[p.id].keys.decrypt[*pos].clone())
                .collect();
            state
                .apply(&Action::RevealHand {
                    player_id: p.id,
                    scalars,
                })
                .unwrap();
        }
    }
    assert!(matches!(state.phase, Phase::Complete));
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
                    "seed={}: action {:?} failed in phase {:?}: {}",
                    seed,
                    std::mem::discriminant(&action),
                    state.phase,
                    e
                );
            });

            steps += 1;
            if steps > max_steps {
                panic!(
                    "seed={}: exceeded {} steps, stuck in phase {:?}",
                    seed, max_steps, state.phase
                );
            }
        }
        eprintln!("seed={}: completed in {} steps ({} players)", seed, steps, num_players);
    }
}

/// Generate a concrete Action from a ValidAction description.
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
            let shuffled =
                players[va.player_id].encrypt_and_shuffle(&state.game.deck, rng);
            Action::ShuffleDeck {
                player_id: va.player_id,
                deck: shuffled,
            }
        }
        ValidActionKind::DecryptCard { position } => Action::DecryptCard {
            player_id: va.player_id,
            position: *position,
            scalar: players[va.player_id].keys.decrypt[*position].clone(),
        },
        ValidActionKind::Bet { options } => {
            let action = options[rng.random_range(0..options.len())].clone();
            Action::Bet {
                player_id: va.player_id,
                action,
            }
        }
        ValidActionKind::RevealHand => {
            let hole_positions: Vec<usize> = (0..2)
                .map(|card_idx| state.deck_position_for_hole_public(va.player_id, card_idx))
                .collect();
            let scalars: Vec<_> = hole_positions
                .iter()
                .map(|pos| players[va.player_id].keys.decrypt[*pos].clone())
                .collect();
            Action::RevealHand {
                player_id: va.player_id,
                scalars,
            }
        }
    }
}

/// Play through a betting round by having everyone check/call.
fn play_betting_round(state: &mut ProtocolState) {
    while matches!(state.phase, Phase::Betting) {
        let actions = state.valid_actions();
        if actions.is_empty() {
            break;
        }
        if let ValidActionKind::Bet { options } = &actions[0].kind {
            // Prefer check, then call
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

/// Deal community cards by having all players provide decryption shares.
fn deal_community_cards(state: &mut ProtocolState, players: &[SimPlayer]) {
    while matches!(state.phase, Phase::DealCommunity { .. }) {
        let actions = state.valid_actions();
        if actions.is_empty() {
            break;
        }
        for va in &actions {
            if let ValidActionKind::DecryptCard { position } = &va.kind {
                state
                    .apply(&Action::DecryptCard {
                        player_id: va.player_id,
                        position: *position,
                        scalar: players[va.player_id].keys.decrypt[*position].clone(),
                    })
                    .unwrap();
            }
        }
    }
}
