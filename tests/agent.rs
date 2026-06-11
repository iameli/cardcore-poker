//! Test the CBOR-in, CBOR-out player agent interface.

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

fn relay_until_stuck(
    alice: &mut PlayerAgent,
    bob: &mut PlayerAgent,
) -> (Option<Vec<BetAction>>, Option<Vec<BetAction>>) {
    relay_until_stuck_with_queues(alice, bob, Vec::new(), Vec::new())
}

fn relay_until_stuck_with_queues(
    alice: &mut PlayerAgent,
    bob: &mut PlayerAgent,
    mut for_bob: Vec<Vec<u8>>,
    mut for_alice: Vec<Vec<u8>>,
) -> (Option<Vec<BetAction>>, Option<Vec<BetAction>>) {
    let max_iters = 1000;

    // Kick off auto-respond
    collect_output(alice.auto_respond_if_needed().unwrap(), &mut for_bob);
    collect_output(bob.auto_respond_if_needed().unwrap(), &mut for_alice);

    for _ in 0..max_iters {
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

        // Feed one action at a time to maintain ordering
        if let Some(action) = for_bob.first().cloned() {
            for_bob.remove(0);
            collect_output(bob.receive_action(&action).unwrap(), &mut for_alice);
        }
        if let Some(action) = for_alice.first().cloned() {
            for_alice.remove(0);
            collect_output(alice.receive_action(&action).unwrap(), &mut for_bob);
        }
    }
    panic!("relay exceeded max iterations");
}

fn collect_output(output: AgentOutput, queue: &mut Vec<Vec<u8>>) {
    if let AgentOutput::Actions(actions) = output {
        queue.extend(actions);
    }
}

#[test]
fn two_agents_full_hand() {
    let alice_did = "did:plc:alice";
    let bob_did = "did:plc:bob";

    let mut alice = PlayerAgent::new(alice_did, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(bob_did, b"bob_seed").unwrap();

    let table_cbor = make_table_cbor(&[alice_did, bob_did], 1000, 10);

    // Both receive the table
    let alice_out = alice.receive_table(&table_cbor).unwrap();
    let bob_out = bob.receive_table(&table_cbor).unwrap();

    // Each should emit commitSeed
    let alice_commit = unwrap_actions(alice_out);
    let bob_commit = unwrap_actions(bob_out);
    assert_eq!(alice_commit.len(), 1);
    assert_eq!(bob_commit.len(), 1);

    // Feed commits to each other — the relay will handle all subsequent phases
    let alice_post_commit = alice.receive_action(&bob_commit[0]).unwrap();
    let bob_post_commit = bob.receive_action(&alice_commit[0]).unwrap();

    // Seed the relay with any immediate responses
    let mut for_bob: Vec<Vec<u8>> = match alice_post_commit {
        AgentOutput::Actions(a) => a,
        _ => vec![],
    };
    // Include the initial commits we already generated
    let mut for_alice: Vec<Vec<u8>> = match bob_post_commit {
        AgentOutput::Actions(a) => a,
        _ => vec![],
    };

    let (a_bet, b_bet) = relay_until_stuck_with_queues(&mut alice, &mut bob, for_bob, for_alice);

    // Should have hole cards now
    eprintln!(
        "After relay - Alice phase: {:?}, Bob phase: {:?}",
        alice.phase(),
        bob.phase()
    );
    eprintln!("Alice hole encrypted: {}", alice.phase() == alice.phase()); // just to force evaluation
    assert_eq!(
        alice.hole_cards().len(),
        2,
        "alice should have 2 hole cards"
    );
    assert_eq!(bob.hole_cards().len(), 2, "bob should have 2 hole cards");
    eprintln!("Alice: {:?}", alice.hole_cards());
    eprintln!("Bob: {:?}", bob.hole_cards());

    // Play through all betting rounds: check/call everything
    let mut a_opts = a_bet;
    let mut b_opts = b_bet;

    for round in 0..100 {
        if matches!(alice.phase(), Phase::Complete) {
            break;
        }
        if round > 50 {
            panic!("too many rounds, phase: {:?}", alice.phase());
        }

        let mut for_bob = Vec::new();
        let mut for_alice = Vec::new();

        if let Some(options) = a_opts.take() {
            let bet = pick_passive(&options);
            if let AgentOutput::Actions(actions) = alice.bet(bet).unwrap() {
                for_bob.extend(actions);
            }
        } else if let Some(options) = b_opts.take() {
            let bet = pick_passive(&options);
            if let AgentOutput::Actions(actions) = bob.bet(bet).unwrap() {
                for_alice.extend(actions);
            }
        }

        let (a, b) = relay_until_stuck_with_queues(&mut alice, &mut bob, for_bob, for_alice);
        a_opts = a;
        b_opts = b;
    }

    eprintln!("Community: {:?}", alice.community_cards());
    eprintln!("Alice phase: {:?}", alice.phase());
    assert!(matches!(alice.phase(), Phase::Complete));
}

fn pick_passive(options: &[BetAction]) -> BetAction {
    if options.iter().any(|o| matches!(o, BetAction::Check)) {
        BetAction::Check
    } else {
        BetAction::Call
    }
}

/// A spectator (DID not in the roster) can replay the full public transcript
/// of a hand: it tracks the whole game, sees community cards and the final
/// settlement, but never gets hole cards before showdown and never emits.
#[test]
fn spectator_replays_full_hand() {
    let alice_did = "did:plc:alice";
    let bob_did = "did:plc:bob";
    let mut alice = PlayerAgent::new(alice_did, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(bob_did, b"bob_seed").unwrap();
    let table_cbor = make_table_cbor(&[alice_did, bob_did], 1000, 10);

    // Play a full passive hand, recording every emitted action in order — the
    // same transcript a spectator would assemble from the players' PDSes.
    let mut transcript: Vec<Vec<u8>> = Vec::new();
    let mut for_alice: Vec<Vec<u8>> = Vec::new();
    let mut for_bob: Vec<Vec<u8>> = Vec::new();

    let record = |out: AgentOutput, transcript: &mut Vec<Vec<u8>>, inbox: &mut Vec<Vec<u8>>| {
        if let AgentOutput::Actions(actions) = out {
            for a in actions {
                transcript.push(a.clone());
                inbox.push(a);
            }
        }
    };

    record(
        alice.receive_table(&table_cbor).unwrap(),
        &mut transcript,
        &mut for_bob,
    );
    record(
        bob.receive_table(&table_cbor).unwrap(),
        &mut transcript,
        &mut for_alice,
    );

    for _ in 0..2000 {
        if matches!(alice.phase(), Phase::Complete) && matches!(bob.phase(), Phase::Complete) {
            break;
        }
        if let Some(action) = (!for_alice.is_empty()).then(|| for_alice.remove(0)) {
            record(
                alice.receive_action(&action).unwrap(),
                &mut transcript,
                &mut for_bob,
            );
            continue;
        }
        if let Some(action) = (!for_bob.is_empty()).then(|| for_bob.remove(0)) {
            record(
                bob.receive_action(&action).unwrap(),
                &mut transcript,
                &mut for_alice,
            );
            continue;
        }
        // Queues drained — somebody must need to bet.
        if let AgentOutput::NeedBet { options } = alice.auto_respond_if_needed().unwrap() {
            record(
                alice.bet(pick_passive(&options)).unwrap(),
                &mut transcript,
                &mut for_bob,
            );
        } else if let AgentOutput::NeedBet { options } = bob.auto_respond_if_needed().unwrap() {
            record(
                bob.bet(pick_passive(&options)).unwrap(),
                &mut transcript,
                &mut for_alice,
            );
        }
    }
    assert!(matches!(alice.phase(), Phase::Complete));

    // Replay the transcript into a spectator who isn't at the table.
    let mut watcher = PlayerAgent::new("did:plc:watcher", b"watcher_seed").unwrap();
    let out = watcher.receive_table(&table_cbor).unwrap();
    assert!(
        matches!(out, AgentOutput::Waiting),
        "spectator must not emit on receive_table"
    );

    for (i, action) in transcript.iter().enumerate() {
        let out = watcher.receive_action(action).unwrap();
        assert!(
            !matches!(out, AgentOutput::Actions(ref a) if !a.is_empty()),
            "spectator emitted an action at transcript step {}",
            i
        );
        // Hole cards stay hidden from the spectator throughout.
        assert_eq!(watcher.hole_cards().len(), 0);
    }

    assert!(matches!(watcher.phase(), Phase::Complete));
    assert_eq!(watcher.community_cards().len(), 5);
    assert!(
        watcher.last_hand_result_json().is_some(),
        "spectator should see the settlement result"
    );
}

fn unwrap_actions(output: AgentOutput) -> Vec<Vec<u8>> {
    match output {
        AgentOutput::Actions(a) => a,
        other => panic!("expected Actions, got {:?}", other),
    }
}

/// Three players; one busts in hand 1; hand 2 plays to completion WITHOUT the
/// busted agent ever being fed another action (as if they closed their tab).
/// This is the busted-player resilience guarantee: dead seats leave the
/// shuffle/lock/reveal protocol entirely.
#[test]
fn game_continues_without_busted_player() {
    let dids = ["did:plc:p0", "did:plc:p1", "did:plc:p2"];
    let mut agents: Vec<PlayerAgent> = dids
        .iter()
        .enumerate()
        .map(|(i, d)| PlayerAgent::new(d, format!("bust_seed_{}", i).as_bytes()).unwrap())
        .collect();
    let table_cbor = make_table_cbor(&dids, 1000, 10);

    fn broadcast(queues: &mut [Vec<Vec<u8>>], active: &[usize], from: usize, actions: &[Vec<u8>]) {
        for &i in active {
            if i != from {
                queues[i].extend(actions.iter().cloned());
            }
        }
    }

    /// Pump messages + scripted bets until every active agent is Complete.
    fn run_hand(
        agents: &mut [PlayerAgent],
        queues: &mut [Vec<Vec<u8>>],
        active: &[usize],
        mut pick: impl FnMut(usize, &[BetAction]) -> BetAction,
    ) {
        let mut bets_made = 0usize;
        for _ in 0..5000 {
            let mut progress = false;
            for &i in active {
                let queue = std::mem::take(&mut queues[i]);
                for action in &queue {
                    if let AgentOutput::Actions(out) = agents[i].receive_action(action).unwrap() {
                        broadcast(queues, active, i, &out);
                    }
                    progress = true;
                }
            }
            if active
                .iter()
                .all(|&i| matches!(agents[i].phase(), Phase::Complete))
            {
                return;
            }
            if !progress {
                let mut bet_done = false;
                for &i in active {
                    if let AgentOutput::NeedBet { options } =
                        agents[i].auto_respond_if_needed().unwrap()
                    {
                        let bet = pick(bets_made, &options);
                        bets_made += 1;
                        if let AgentOutput::Actions(out) = agents[i].bet(bet).unwrap() {
                            broadcast(queues, active, i, &out);
                        }
                        bet_done = true;
                        break;
                    }
                }
                if !bet_done {
                    panic!(
                        "hand stalled; phases: {:?}",
                        active
                            .iter()
                            .map(|&i| format!("{:?}", agents[i].phase()))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        panic!("hand exceeded max iterations");
    }

    // Hand 1: everyone at the table; first bettor shoves, second folds, third
    // calls — the all-in loser busts (deterministic deal from fixed seeds).
    let mut queues: Vec<Vec<Vec<u8>>> = vec![vec![]; 3];
    let everyone = [0usize, 1, 2];
    for i in 0..3 {
        let out = agents[i].receive_table(&table_cbor).unwrap();
        if let AgentOutput::Actions(a) = out {
            broadcast(&mut queues, &everyone, i, &a);
        }
    }
    run_hand(&mut agents, &mut queues, &everyone, |n, options| match n {
        0 => BetAction::AllIn,
        1 => BetAction::Fold,
        _ => {
            if options.iter().any(|o| matches!(o, BetAction::Call)) {
                BetAction::Call
            } else if options.iter().any(|o| matches!(o, BetAction::Check)) {
                BetAction::Check
            } else {
                BetAction::AllIn
            }
        }
    });

    // Find who busted (the all-in pot can't tie with these seeds).
    let gs: serde_json::Value = serde_json::from_str(&agents[0].game_state_json()).unwrap();
    let chips: Vec<u64> = gs["players"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["chips"].as_u64().unwrap())
        .collect();
    let busted = chips
        .iter()
        .position(|&c| c == 0)
        .expect("expected a busted player (re-pick seeds if the pot tied)");
    let survivors: Vec<usize> = (0..3).filter(|&i| i != busted).collect();
    assert!(!agents[0].game_over(), "two survivors should keep playing");

    // Hand 2: the busted player's tab is CLOSED — their agent is never fed
    // again, and the survivors' agents must not need anything from it.
    for &i in &survivors {
        let out = agents[i].next_hand().unwrap();
        if let AgentOutput::Actions(a) = out {
            broadcast(&mut queues, &survivors, i, &a);
        }
    }
    run_hand(&mut agents, &mut queues, &survivors, |_, options| {
        if options.iter().any(|o| matches!(o, BetAction::Check)) {
            BetAction::Check
        } else {
            BetAction::Call
        }
    });

    for &i in &survivors {
        assert!(matches!(agents[i].phase(), Phase::Complete));
        assert_eq!(agents[i].hole_cards().len(), 2);
    }
}

#[test]
fn waiting_on_names_the_pending_step_and_seats() {
    let alice_did = "did:plc:alice";
    let bob_did = "did:plc:bob";

    let mut alice = PlayerAgent::new(alice_did, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(bob_did, b"bob_seed").unwrap();

    let table_cbor = make_table_cbor(&[alice_did, bob_did], 1000, 10);

    // Both receive the table and commit their own seeds. From bob's view the
    // protocol is now blocked on alice's commitSeed (his own is already in).
    let alice_commit = unwrap_actions(alice.receive_table(&table_cbor).unwrap());
    let bob_commit = unwrap_actions(bob.receive_table(&table_cbor).unwrap());
    assert_eq!(alice_commit.len(), 1);
    assert_eq!(bob_commit.len(), 1);

    let waiting: serde_json::Value = serde_json::from_str(&bob.waiting_on_json()).unwrap();
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    assert_eq!(waiting[0]["kind"], "commitSeed");
    assert_eq!(waiting[0]["seats"], serde_json::json!([0]));

    // Alice's commit arrives: all seeds are in, and seat 0 (alice) owes the
    // first shuffle. This is the exact "everyone committed but nobody
    // shuffled" stall — bob's window should say
    // "waiting on shuffleDeck from alice".
    let _ = bob.receive_action(&alice_commit[0]).unwrap();

    let waiting: serde_json::Value = serde_json::from_str(&bob.waiting_on_json()).unwrap();
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    assert_eq!(waiting[0]["kind"], "shuffleDeck");
    assert_eq!(waiting[0]["seats"], serde_json::json!([0]));
}
