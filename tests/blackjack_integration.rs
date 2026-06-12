//! Blackjack protocol integration tests: full provably-fair rounds over the
//! shared two-phase shuffle+lock engine, plus fuzzing.

use cardcore_poker::blackjack::game::Decision;
use cardcore_poker::blackjack::protocol::{BjAction, BjPhase, BjProtocolState, BjValidActionKind};
use cardcore_poker::crypto::{self, PlayerKeys, PlayerRng, Point};
use rand::prelude::*;
use rand::rngs::StdRng;

struct SimPlayer {
    id: usize,
    seed: Vec<u8>,
    keys: PlayerKeys,
}

impl SimPlayer {
    fn new(id: usize) -> Self {
        crypto::init().unwrap();
        let seed = format!("bj_player_{}_seed_{}", id, id * 54321).into_bytes();
        let mut rng = PlayerRng::new(&seed, b"shuffle").unwrap();
        let keys = PlayerKeys::generate(&mut rng).unwrap();
        Self { id, seed, keys }
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

fn setup(n: usize) -> (BjProtocolState, Vec<SimPlayer>) {
    let mut state = BjProtocolState::new();
    let players: Vec<SimPlayer> = (0..n).map(SimPlayer::new).collect();
    state
        .apply(&BjAction::Table {
            players: (0..n).map(|i| format!("did:example:player{}", i)).collect(),
            starting_chips: 1000,
            min_bet: 10,
        })
        .unwrap();
    (state, players)
}

/// Re-derive each live player's keys for a fresh round (each round the deck
/// is shuffled and locked anew, so lock keys are regenerated on demand).
fn rederive_keys(players: &mut [SimPlayer]) {
    for p in players.iter_mut() {
        let mut rng = PlayerRng::new(&p.seed, b"shuffle").unwrap();
        p.keys = PlayerKeys::generate(&mut rng).unwrap();
    }
}

/// Build the action a player would take for the given valid action.
/// `decide` picks among legal decisions; wagers use the minimum.
fn respond(
    kind: &BjValidActionKind,
    player_id: usize,
    state: &BjProtocolState,
    players: &mut [SimPlayer],
    decide: &mut dyn FnMut(&[Decision]) -> Decision,
) -> BjAction {
    match kind {
        BjValidActionKind::CommitSeed => BjAction::CommitSeed {
            player_id,
            commitment: players[player_id].commitment(),
        },
        BjValidActionKind::ShuffleDeck => BjAction::ShuffleDeck {
            player_id,
            deck: players[player_id].encrypt_and_shuffle(&state.crypto.deck),
        },
        BjValidActionKind::LockDeck => BjAction::LockDeck {
            player_id,
            deck: players[player_id].lock_deck(&state.crypto.deck),
        },
        BjValidActionKind::RevealLockKey { deck_position } => BjAction::RevealLockKey {
            player_id,
            deck_position: *deck_position,
            scalar: players[player_id].keys.lock_decrypt[*deck_position].clone(),
        },
        BjValidActionKind::Wager { min, .. } => BjAction::Wager {
            player_id,
            amount: *min,
        },
        BjValidActionKind::Insurance => BjAction::Insurance {
            player_id,
            take: false,
        },
        BjValidActionKind::Decision { options } => BjAction::Decision {
            player_id,
            decision: decide(options),
        },
        BjValidActionKind::VerifySeed => BjAction::VerifySeed {
            player_id,
            seed: players[player_id].seed.clone(),
        },
    }
}

/// Drive the round to `Complete` by always applying the first valid action.
fn drive_round(
    state: &mut BjProtocolState,
    players: &mut [SimPlayer],
    decide: &mut dyn FnMut(&[Decision]) -> Decision,
) {
    let mut steps = 0;
    while state.phase != BjPhase::Complete {
        let valid = state.valid_actions();
        assert!(
            !valid.is_empty(),
            "no valid actions in phase {:?}",
            state.phase
        );
        let va = valid[0].clone();
        let action = respond(&va.kind, va.player_id, state, players, decide);
        state
            .apply(&action)
            .unwrap_or_else(|e| panic!("action failed in {:?}: {}", state.phase, e));
        steps += 1;
        assert!(steps < 5000, "round did not complete");
    }
}

#[test]
fn full_round_with_seed_verification() {
    let (mut state, mut players) = setup(3);
    let mut stand = |_: &[Decision]| Decision::Stand;
    drive_round(&mut state, &mut players, &mut stand);

    // Banker completed to >= 17 (or bust) and the result is recorded.
    let result = state.last_round_result.as_ref().unwrap();
    assert!(result.banker.total >= 17);
    assert_eq!(result.banker.seat, 0);
    // Bettors stood on two face-up cards each.
    for pid in 1..3 {
        assert_eq!(state.game.players[pid].hands.len(), 1);
        assert_eq!(state.game.players[pid].hands[0].cards.len(), 2);
    }
    // Chips conserved across the table.
    let total: u64 = state.game.players.iter().map(|p| p.chips).sum();
    assert_eq!(total, 3000);

    // Post-game: everyone reveals their seed and verifies.
    for p in &players {
        state
            .apply(&BjAction::VerifySeed {
                player_id: p.id,
                seed: p.seed.clone(),
            })
            .unwrap();
    }
    assert!(state.crypto.seeds_verified.iter().all(|v| *v));

    // Wrong seed must be rejected (nothing left to verify anyway).
    assert!(
        state
            .apply(&BjAction::VerifySeed {
                player_id: 0,
                seed: b"forged".to_vec(),
            })
            .is_err()
    );
}

#[test]
fn multi_round_rotates_banker_and_reshuffles() {
    let (mut state, mut players) = setup(2);
    let mut stand = |_: &[Decision]| Decision::Stand;

    for round in 0..3u64 {
        assert_eq!(state.hand_index, round);
        let expected_banker = (round as usize) % 2;
        assert_eq!(state.game.banker, expected_banker);
        drive_round(&mut state, &mut players, &mut stand);
        let total: u64 = state.game.players.iter().map(|p| p.chips).sum();
        assert_eq!(total, 2000, "chips conserved after round {}", round);
        if state.game_over() {
            return;
        }
        state.start_next_round();
        rederive_keys(&mut players);
    }
}

#[test]
fn invalid_actions_are_rejected() {
    let (mut state, mut players) = setup(3);

    // Out-of-phase wager.
    assert!(
        state
            .apply(&BjAction::Wager {
                player_id: 1,
                amount: 10
            })
            .is_err()
    );

    // Crypto setup.
    let mut stand = |_: &[Decision]| Decision::Stand;
    while !matches!(state.phase, BjPhase::Wagering { .. }) {
        let valid = state.valid_actions();
        let va = valid[0].clone();
        let action = respond(&va.kind, va.player_id, &state, &mut players, &mut stand);
        state.apply(&action).unwrap();
    }

    // Out-of-turn and out-of-bounds wagers.
    assert!(
        state
            .apply(&BjAction::Wager {
                player_id: 2,
                amount: 10
            })
            .is_err()
    );
    assert!(
        state
            .apply(&BjAction::Wager {
                player_id: 1,
                amount: 5
            })
            .is_err()
    );
    assert!(
        state
            .apply(&BjAction::Wager {
                player_id: 1,
                amount: 100_000
            })
            .is_err()
    );
    state
        .apply(&BjAction::Wager {
            player_id: 1,
            amount: 10,
        })
        .unwrap();
    state
        .apply(&BjAction::Wager {
            player_id: 2,
            amount: 10,
        })
        .unwrap();

    // Dealing: wrong deck position and double reveals are rejected.
    let (revealer, pos) = match &state.phase {
        BjPhase::Dealing { deck_position, .. } => {
            (state.valid_actions()[0].player_id, *deck_position)
        }
        other => panic!("expected dealing, got {:?}", other),
    };
    assert!(
        state
            .apply(&BjAction::RevealLockKey {
                player_id: revealer,
                deck_position: pos + 1,
                scalar: players[revealer].keys.lock_decrypt[pos].clone(),
            })
            .is_err()
    );
    state
        .apply(&BjAction::RevealLockKey {
            player_id: revealer,
            deck_position: pos,
            scalar: players[revealer].keys.lock_decrypt[pos].clone(),
        })
        .unwrap();
    assert!(
        state
            .apply(&BjAction::RevealLockKey {
                player_id: revealer,
                deck_position: pos,
                scalar: players[revealer].keys.lock_decrypt[pos].clone(),
            })
            .is_err(),
        "double reveal must be rejected"
    );

    // Out-of-turn decision while still dealing.
    assert!(
        state
            .apply(&BjAction::Decision {
                player_id: 1,
                decision: Decision::Hit
            })
            .is_err()
    );
}

#[test]
fn eliminated_players_are_excluded_from_the_protocol() {
    let (mut state, mut players) = setup(3);
    let mut stand = |_: &[Decision]| Decision::Stand;
    drive_round(&mut state, &mut players, &mut stand);

    // Bust seat 1 by hand (their chips leave the economy) and start the next
    // round.
    state.game.players[1].chips = 0;
    let expected_total: u64 = state.game.players.iter().map(|p| p.chips).sum();
    assert!(!state.game_over());
    state.start_next_round();
    rederive_keys(&mut players);
    assert!(state.game.players[1].eliminated);
    assert_eq!(
        state.game.banker, 2,
        "banker rotation skips the busted seat"
    );

    // Only live seats may commit.
    let committers: Vec<usize> = state.valid_actions().iter().map(|v| v.player_id).collect();
    assert_eq!(committers, vec![0, 2]);
    assert!(
        state
            .apply(&BjAction::CommitSeed {
                player_id: 1,
                commitment: players[1].commitment(),
            })
            .is_err(),
        "eliminated player's commit must be rejected"
    );

    // The remaining seats play a full round without the busted one.
    drive_round(&mut state, &mut players, &mut stand);
    assert!(state.game.players[1].hands.is_empty());
    let total: u64 = state.game.players.iter().map(|p| p.chips).sum();
    assert_eq!(total, expected_total);
}

/// Fuzz: random (but valid) actions — random wagers, random decisions
/// including doubles/splits/surrenders, random insurance — across several
/// rounds. The protocol must never panic and chips must always balance.
#[test]
fn fuzz_random_valid_actions() {
    for seed in 0..10u64 {
        let mut rng = StdRng::seed_from_u64(seed);
        let num_players = rng.random_range(2..=4);
        let (mut state, mut players) = setup(num_players);
        let starting_total = (num_players as u64) * 1000;

        for _round in 0..3 {
            let mut steps = 0;
            while state.phase != BjPhase::Complete {
                let valid = state.valid_actions();
                assert!(
                    !valid.is_empty(),
                    "seed={}: no valid actions in {:?}",
                    seed,
                    state.phase
                );
                let va = valid[rng.random_range(0..valid.len())].clone();
                let action = match &va.kind {
                    BjValidActionKind::Wager { min, max } => {
                        // Keep wagers smallish so games last several rounds.
                        let hi = (*min + 50).min(*max);
                        BjAction::Wager {
                            player_id: va.player_id,
                            amount: rng.random_range(*min..=hi),
                        }
                    }
                    BjValidActionKind::Insurance => BjAction::Insurance {
                        player_id: va.player_id,
                        take: rng.random_bool(0.5),
                    },
                    BjValidActionKind::Decision { options } => BjAction::Decision {
                        player_id: va.player_id,
                        decision: options[rng.random_range(0..options.len())],
                    },
                    other => {
                        let mut pick = |opts: &[Decision]| opts[0];
                        respond(other, va.player_id, &state, &mut players, &mut pick)
                    }
                };
                state.apply(&action).unwrap_or_else(|e| {
                    panic!("seed={}: action failed in {:?}: {}", seed, state.phase, e)
                });
                steps += 1;
                assert!(steps < 10_000, "seed={}: runaway round", seed);
            }

            let total: u64 = state.game.players.iter().map(|p| p.chips).sum();
            assert_eq!(total, starting_total, "seed={}: chips must balance", seed);

            // Everyone verifies their seed after the round.
            for p in players.iter() {
                if !state.game.players[p.id].eliminated {
                    state
                        .apply(&BjAction::VerifySeed {
                            player_id: p.id,
                            seed: p.seed.clone(),
                        })
                        .unwrap();
                }
            }

            if state.game_over() {
                break;
            }
            state.start_next_round();
            rederive_keys(&mut players);
        }
    }
}
