//! WASM browser tests — run via `wasm-pack test --headless --chrome`.
//!
//! These exercise the full WasmAgent API in a real browser engine,
//! same flow as the native agent tests but through the wasm-bindgen interface.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

use cardcore_poker::agent::{AgentOutput, PlayerAgent};
use cardcore_poker::game::BetAction;
use cardcore_poker::lexicon::re_cardco::poker::table::Table as LexTable;
use cardcore_poker::protocol::Phase;
use jacquard_common::types::string::Datetime;

fn make_table_cbor(dids: &[&str], chips: i64, sb: i64) -> Vec<u8> {
    let table = LexTable {
        players: dids.iter().map(|d| d.to_string().into()).collect(),
        starting_chips: chips,
        small_blind: sb,
        created_at: Datetime::now(),
        extra_data: None,
    };
    dasl::drisl::to_vec(&table).unwrap()
}

fn collect_actions(output: AgentOutput) -> Vec<Vec<u8>> {
    match output {
        AgentOutput::Actions(a) => a,
        _ => vec![],
    }
}

/// Relay actions between two agents until both idle or one needs a bet.
fn relay(
    alice: &mut PlayerAgent,
    bob: &mut PlayerAgent,
    mut for_bob: Vec<Vec<u8>>,
    mut for_alice: Vec<Vec<u8>>,
) -> (Option<Vec<BetAction>>, Option<Vec<BetAction>>) {
    // Kick off auto-respond
    if let AgentOutput::Actions(a) = alice.auto_respond_if_needed().unwrap() {
        for_bob.extend(a);
    }
    if let AgentOutput::Actions(a) = bob.auto_respond_if_needed().unwrap() {
        for_alice.extend(a);
    }

    for _ in 0..1000 {
        if for_bob.is_empty() && for_alice.is_empty() {
            let a = match alice.auto_respond_if_needed().unwrap() {
                AgentOutput::NeedBet { options } => Some(options),
                AgentOutput::Actions(a) => {
                    for_bob.extend(a);
                    None
                }
                AgentOutput::Waiting => None,
            };
            let b = match bob.auto_respond_if_needed().unwrap() {
                AgentOutput::NeedBet { options } => Some(options),
                AgentOutput::Actions(a) => {
                    for_alice.extend(a);
                    None
                }
                AgentOutput::Waiting => None,
            };
            if a.is_some() || b.is_some() || (for_bob.is_empty() && for_alice.is_empty()) {
                return (a, b);
            }
            continue;
        }

        if let Some(action) = for_bob.first().cloned() {
            for_bob.remove(0);
            if let AgentOutput::Actions(a) = bob.receive_action(&action).unwrap() {
                for_alice.extend(a);
            }
        }
        if let Some(action) = for_alice.first().cloned() {
            for_alice.remove(0);
            if let AgentOutput::Actions(a) = alice.receive_action(&action).unwrap() {
                for_bob.extend(a);
            }
        }
    }
    panic!("relay exceeded max iterations");
}

#[wasm_bindgen_test]
fn crypto_roundtrip_in_browser() {
    // Basic sanity: crypto works in WASM
    let (enc, dec) = cardcore_poker::crypto::generate_keypair().unwrap();
    let card = cardcore_poker::card::Card::new(
        cardcore_poker::card::Rank::Ace,
        cardcore_poker::card::Suit::Spades,
    );
    let point = cardcore_poker::crypto::card_to_point(&card).unwrap();
    let encrypted = cardcore_poker::crypto::encrypt(&point, &enc).unwrap();
    assert_ne!(encrypted, point);
    let decrypted = cardcore_poker::crypto::decrypt(&encrypted, &dec).unwrap();
    assert_eq!(decrypted, point);
}

#[wasm_bindgen_test]
fn two_agents_full_hand_in_browser() {
    let alice_did = "did:plc:alice";
    let bob_did = "did:plc:bob";

    let mut alice = PlayerAgent::new(alice_did, b"alice_wasm_seed").unwrap();
    let mut bob = PlayerAgent::new(bob_did, b"bob_wasm_seed").unwrap();

    let table_cbor = make_table_cbor(&[alice_did, bob_did], 1000, 10);

    // Both receive table → emit commitSeed
    let alice_commit = collect_actions(alice.receive_table(&table_cbor).unwrap());
    let bob_commit = collect_actions(bob.receive_table(&table_cbor).unwrap());
    assert_eq!(alice_commit.len(), 1);
    assert_eq!(bob_commit.len(), 1);

    // Exchange commits
    let for_bob = collect_actions(alice.receive_action(&bob_commit[0]).unwrap());
    let for_alice = collect_actions(bob.receive_action(&alice_commit[0]).unwrap());

    // Relay through shuffle, lock, deal
    let (a_bet, b_bet) = relay(&mut alice, &mut bob, for_bob, for_alice);

    // Should have hole cards
    assert_eq!(alice.hole_cards().len(), 2);
    assert_eq!(bob.hole_cards().len(), 2);

    // Play through betting: check/call everything
    let mut a_opts = a_bet;
    let mut b_opts = b_bet;

    for _ in 0..100 {
        if matches!(alice.phase(), Phase::Complete) {
            break;
        }

        let mut for_bob = Vec::new();
        let mut for_alice = Vec::new();

        if let Some(options) = a_opts.take() {
            let bet = passive_bet(&options);
            if let AgentOutput::Actions(a) = alice.bet(bet).unwrap() {
                for_bob.extend(a);
            }
        } else if let Some(options) = b_opts.take() {
            let bet = passive_bet(&options);
            if let AgentOutput::Actions(a) = bob.bet(bet).unwrap() {
                for_alice.extend(a);
            }
        }

        let (a, b) = relay(&mut alice, &mut bob, for_bob, for_alice);
        a_opts = a;
        b_opts = b;
    }

    assert!(
        matches!(alice.phase(), Phase::Complete),
        "game should be complete, got {:?}",
        alice.phase()
    );

    // Verify we got community cards
    let community = alice.community_cards();
    assert_eq!(community.len(), 5, "should have 5 community cards");
}

#[wasm_bindgen_test]
#[ignore] // 3-player relay works natively (sim tests) but this test's relay logic needs updating
fn three_player_hand_in_browser() {
    let dids = ["did:plc:a", "did:plc:b", "did:plc:c"];
    let mut agents: Vec<PlayerAgent> = dids
        .iter()
        .enumerate()
        .map(|(i, did)| {
            PlayerAgent::new(did, format!("seed_{}", i).as_bytes()).unwrap()
        })
        .collect();

    let table_cbor = make_table_cbor(&dids, 1000, 10);

    // All receive table
    let commits: Vec<Vec<Vec<u8>>> = agents
        .iter_mut()
        .map(|a| collect_actions(a.receive_table(&table_cbor).unwrap()))
        .collect();

    // Exchange commits: each agent receives the other two
    for i in 0..3 {
        for j in 0..3 {
            if i != j {
                for action in &commits[j] {
                    let _ = agents[i].receive_action(action);
                }
            }
        }
    }

    // Now relay between all three agents until betting
    // Simple approach: round-robin collecting and distributing actions
    let mut queues: Vec<Vec<Vec<u8>>> = vec![vec![]; 3];

    // Initial auto-respond
    for i in 0..3 {
        if let AgentOutput::Actions(a) = agents[i].auto_respond_if_needed().unwrap() {
            // Send to all other agents
            for j in 0..3 {
                if j != i {
                    queues[j].extend(a.clone());
                }
            }
        }
    }

    for _ in 0..2000 {
        let mut any_progress = false;

        for i in 0..3 {
            let my_queue = std::mem::take(&mut queues[i]);
            for action in &my_queue {
                any_progress = true;
                if let AgentOutput::Actions(responses) = agents[i].receive_action(action).unwrap()
                {
                    for j in 0..3 {
                        if j != i {
                            queues[j].extend(responses.clone());
                        }
                    }
                }
            }
        }

        // Auto-respond for anyone who can
        for i in 0..3 {
            if let AgentOutput::Actions(a) = agents[i].auto_respond_if_needed().unwrap() {
                if !a.is_empty() {
                    any_progress = true;
                    for j in 0..3 {
                        if j != i {
                            queues[j].extend(a.clone());
                        }
                    }
                }
            }
        }

        // Check if anyone needs a bet
        let mut need_bet = None;
        for i in 0..3 {
            if let AgentOutput::NeedBet { options } = agents[i].auto_respond_if_needed().unwrap() {
                need_bet = Some((i, options));
                break;
            }
        }

        if let Some((idx, options)) = need_bet {
            let bet = passive_bet(&options);
            if let AgentOutput::Actions(a) = agents[idx].bet(bet).unwrap() {
                for j in 0..3 {
                    if j != idx {
                        queues[j].extend(a.clone());
                    }
                }
            }
            continue;
        }

        if !any_progress && queues.iter().all(|q| q.is_empty()) {
            break;
        }
    }

    assert!(
        matches!(agents[0].phase(), Phase::Complete),
        "3-player game should complete, got {:?}",
        agents[0].phase()
    );
    assert_eq!(agents[0].community_cards().len(), 5);
    for a in &agents {
        if !matches!(a.phase(), Phase::Complete) {
            panic!("agent not complete: {:?}", a.phase());
        }
    }
}

#[wasm_bindgen_test]
fn simulator_runs_in_browser() {
    use cardcore_poker::sim::{BotStrategy, GameEvent, SimConfig, Simulator};

    let config = SimConfig {
        num_players: 3,
        starting_chips: 1000,
        small_blind: 10,
        strategy: BotStrategy::Passive,
        rng_seed: 77,
    };
    let mut sim = Simulator::new(config).unwrap();
    sim.run().unwrap();

    let events = sim.events();
    assert!(events.iter().any(|e| matches!(e, GameEvent::TableCreated { .. })));
    assert!(events.iter().any(|e| matches!(e, GameEvent::HoleCardsDealt { .. })));
    assert!(events.iter().any(|e| matches!(e, GameEvent::CommunityDealt { street, .. } if street == "flop")));
    assert!(events.iter().any(|e| matches!(e, GameEvent::SeedsVerified)));

    // Verify JSON serialization works in WASM too
    let json = serde_json::to_string(events).unwrap();
    assert!(json.contains("tableCreated"));
}

fn passive_bet(options: &[BetAction]) -> BetAction {
    if options.iter().any(|o| matches!(o, BetAction::Check)) {
        BetAction::Check
    } else {
        BetAction::Call
    }
}
