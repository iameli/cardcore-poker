//! Test the CBOR-in, CBOR-out blackjack agent interface.

use cardcore_poker::blackjack::agent::{BjAgentOutput, BlackjackAgent};
use cardcore_poker::blackjack::game::Decision;
use cardcore_poker::blackjack::protocol::BjPhase;
use cardcore_poker::lexicon::re_cardco::blackjack::table::Table as LexTable;
use jacquard_common::types::string::Datetime;

fn make_table_cbor(dids: &[&str], chips: i64, min_bet: i64) -> Vec<u8> {
    let table = LexTable {
        players: dids.iter().map(|d| d.to_string().into()).collect(),
        starting_chips: chips,
        min_bet,
        started_at: None,
        updated_at: None,
        created_at: Datetime::now(),
        extra_data: None,
    };
    dasl::drisl::to_vec(&table).unwrap()
}

fn broadcast(queues: &mut [Vec<Vec<u8>>], from: usize, actions: &[Vec<u8>]) {
    for (i, q) in queues.iter_mut().enumerate() {
        if i != from {
            q.extend(actions.iter().cloned());
        }
    }
}

/// Feed every agent the table record and seed the queues with the responses.
fn deal_in(agents: &mut [BlackjackAgent], queues: &mut [Vec<Vec<u8>>], table_cbor: &[u8]) {
    for i in 0..agents.len() {
        if let BjAgentOutput::Actions(a) = agents[i].receive_table(table_cbor).unwrap() {
            broadcast(queues, i, &a);
        }
    }
}

/// Drive all agents until the round completes everywhere. Queued messages are
/// delivered one at a time (ordering matters); when everything is drained,
/// interactive needs are answered with a fixed policy: minimum wager, decline
/// insurance, prefer Stand.
fn run_round(agents: &mut [BlackjackAgent], queues: &mut [Vec<Vec<u8>>]) {
    for _ in 0..5000 {
        let mut delivered = false;
        for i in 0..agents.len() {
            if queues[i].is_empty() {
                continue;
            }
            let msg = queues[i].remove(0);
            if let BjAgentOutput::Actions(a) = agents[i].receive_action(&msg).unwrap() {
                broadcast(queues, i, &a);
            }
            delivered = true;
        }
        if delivered {
            continue;
        }

        // Queues drained — let agents act or answer their needs.
        let mut acted = false;
        for i in 0..agents.len() {
            let response = match agents[i].auto_respond_if_needed().unwrap() {
                BjAgentOutput::Actions(a) if !a.is_empty() => Some(a),
                BjAgentOutput::NeedWager { min, .. } => match agents[i].wager(min).unwrap() {
                    BjAgentOutput::Actions(a) => Some(a),
                    _ => None,
                },
                BjAgentOutput::NeedInsurance => match agents[i].insurance(false).unwrap() {
                    BjAgentOutput::Actions(a) => Some(a),
                    _ => None,
                },
                BjAgentOutput::NeedDecision { options } => {
                    let choice = if options.contains(&Decision::Stand) {
                        Decision::Stand
                    } else {
                        options[0]
                    };
                    match agents[i].decide(choice).unwrap() {
                        BjAgentOutput::Actions(a) => Some(a),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(a) = response {
                broadcast(queues, i, &a);
                acted = true;
            }
        }
        if !acted {
            assert!(
                agents
                    .iter()
                    .all(|ag| matches!(ag.phase(), BjPhase::Complete)),
                "relay stuck before Complete"
            );
            return;
        }
    }
    panic!("relay exceeded max iterations");
}

#[test]
fn two_agents_full_round() {
    let dids = ["did:plc:alice", "did:plc:bob"];
    let mut agents = vec![
        BlackjackAgent::new(dids[0], b"alice_seed").unwrap(),
        BlackjackAgent::new(dids[1], b"bob_seed").unwrap(),
    ];
    let mut queues = vec![Vec::new(), Vec::new()];
    deal_in(&mut agents, &mut queues, &make_table_cbor(&dids, 1000, 10));

    run_round(&mut agents, &mut queues);

    // Both agents agree on the final state.
    assert_eq!(agents[0].game_state_json(), agents[1].game_state_json());

    // Alice (seat 0) banked: at least two cards, no hand of her own.
    assert!(agents[0].banker_cards().len() >= 2);
    assert!(agents[0].my_hands().is_empty());
    // Bob stood on his two face-up cards.
    assert_eq!(agents[1].my_hands().len(), 1);
    assert_eq!(agents[1].my_hands()[0].len(), 2);

    // The round result is available everywhere.
    for ag in &agents {
        let result = ag.last_round_result_json().expect("round result");
        assert!(result.contains("banker"));
        assert!(result.contains("chips_after"));
    }

    // Chips balance.
    let state: serde_json::Value = serde_json::from_str(&agents[0].game_state_json()).unwrap();
    let total: u64 = state["players"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["chips"].as_u64().unwrap())
        .sum();
    assert_eq!(total, 2000);
}

#[test]
fn three_agents_full_round() {
    let dids = ["did:plc:alice", "did:plc:bob", "did:plc:carol"];
    let mut agents = vec![
        BlackjackAgent::new(dids[0], b"alice_seed").unwrap(),
        BlackjackAgent::new(dids[1], b"bob_seed").unwrap(),
        BlackjackAgent::new(dids[2], b"carol_seed").unwrap(),
    ];
    let mut queues = vec![Vec::new(), Vec::new(), Vec::new()];
    deal_in(&mut agents, &mut queues, &make_table_cbor(&dids, 1000, 10));

    run_round(&mut agents, &mut queues);

    let reference = agents[0].game_state_json();
    for ag in &agents[1..] {
        assert_eq!(ag.game_state_json(), reference);
    }
    let state: serde_json::Value = serde_json::from_str(&reference).unwrap();
    let total: u64 = state["players"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["chips"].as_u64().unwrap())
        .sum();
    assert_eq!(total, 3000);
    // Both bettors (seats 1 and 2) played a hand against seat 0's bank.
    assert_eq!(state["banker"], 0);
    assert_eq!(state["players"][1]["hands"].as_array().unwrap().len(), 1);
    assert_eq!(state["players"][2]["hands"].as_array().unwrap().len(), 1);
}

#[test]
fn multi_round_game_rotates_banker() {
    let dids = ["did:plc:alice", "did:plc:bob"];
    let mut agents = vec![
        BlackjackAgent::new(dids[0], b"alice_seed").unwrap(),
        BlackjackAgent::new(dids[1], b"bob_seed").unwrap(),
    ];
    let mut queues = vec![Vec::new(), Vec::new()];
    deal_in(&mut agents, &mut queues, &make_table_cbor(&dids, 1000, 10));

    for round in 0..3u64 {
        let state: serde_json::Value = serde_json::from_str(&agents[0].game_state_json()).unwrap();
        assert_eq!(state["handIndex"], round);
        assert_eq!(state["banker"], (round as usize % 2) as u64);

        run_round(&mut agents, &mut queues);

        if agents[0].game_over() {
            return;
        }
        // Everyone advances to the next round; new commits go out.
        for i in 0..agents.len() {
            if let BjAgentOutput::Actions(a) = agents[i].next_round().unwrap() {
                broadcast(&mut queues, i, &a);
            }
        }
    }
}

#[test]
fn spectator_replays_full_round() {
    let dids = ["did:plc:alice", "did:plc:bob"];
    let table_cbor = make_table_cbor(&dids, 1000, 10);
    let mut agents = vec![
        BlackjackAgent::new(dids[0], b"alice_seed").unwrap(),
        BlackjackAgent::new(dids[1], b"bob_seed").unwrap(),
        // Not in the roster: pure observer.
        BlackjackAgent::new("did:plc:watcher", b"watcher_seed").unwrap(),
    ];
    let mut queues = vec![Vec::new(), Vec::new(), Vec::new()];
    deal_in(&mut agents, &mut queues, &table_cbor);

    // The spectator can't act.
    assert!(agents[2].wager(10).is_err());
    assert!(agents[2].my_hands().is_empty());

    run_round(&mut agents, &mut queues);

    // The spectator reconstructed the identical public state.
    assert_eq!(agents[2].game_state_json(), agents[0].game_state_json());
    assert!(matches!(agents[2].phase(), BjPhase::Complete));
    assert!(agents[2].last_round_result_json().is_some());
}
