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

const ALICE_DID: &str = "did:plc:alice";
const BOB_DID: &str = "did:plc:bob";

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

        // Feed one action at a time to maintain ordering. Everything bound
        // for bob was authored by alice, and vice versa.
        if let Some(action) = for_bob.first().cloned() {
            for_bob.remove(0);
            collect_output(
                bob.receive_action(&action, ALICE_DID).unwrap(),
                &mut for_alice,
            );
        }
        if let Some(action) = for_alice.first().cloned() {
            for_alice.remove(0);
            collect_output(
                alice.receive_action(&action, BOB_DID).unwrap(),
                &mut for_bob,
            );
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
    let alice_post_commit = alice.receive_action(&bob_commit[0], BOB_DID).unwrap();
    let bob_post_commit = bob.receive_action(&alice_commit[0], ALICE_DID).unwrap();

    // Seed the relay with any immediate responses
    let for_bob: Vec<Vec<u8>> = match alice_post_commit {
        AgentOutput::Actions(a) => a,
        _ => vec![],
    };
    // Include the initial commits we already generated
    let for_alice: Vec<Vec<u8>> = match bob_post_commit {
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

    // Play a full passive hand, recording every emitted action with its
    // author, in order — the same transcript a spectator would assemble from
    // the players' PDSes.
    let mut transcript: Vec<(&str, Vec<u8>)> = Vec::new();
    let mut for_alice: Vec<Vec<u8>> = Vec::new();
    let mut for_bob: Vec<Vec<u8>> = Vec::new();

    let record = |out: AgentOutput,
                  author: &'static str,
                  transcript: &mut Vec<(&str, Vec<u8>)>,
                  inbox: &mut Vec<Vec<u8>>| {
        if let AgentOutput::Actions(actions) = out {
            for a in actions {
                transcript.push((author, a.clone()));
                inbox.push(a);
            }
        }
    };

    record(
        alice.receive_table(&table_cbor).unwrap(),
        ALICE_DID,
        &mut transcript,
        &mut for_bob,
    );
    record(
        bob.receive_table(&table_cbor).unwrap(),
        BOB_DID,
        &mut transcript,
        &mut for_alice,
    );

    for _ in 0..2000 {
        if matches!(alice.phase(), Phase::Complete) && matches!(bob.phase(), Phase::Complete) {
            break;
        }
        if let Some(action) = (!for_alice.is_empty()).then(|| for_alice.remove(0)) {
            record(
                alice.receive_action(&action, BOB_DID).unwrap(),
                ALICE_DID,
                &mut transcript,
                &mut for_bob,
            );
            continue;
        }
        if let Some(action) = (!for_bob.is_empty()).then(|| for_bob.remove(0)) {
            record(
                bob.receive_action(&action, ALICE_DID).unwrap(),
                BOB_DID,
                &mut transcript,
                &mut for_alice,
            );
            continue;
        }
        // Queues drained — somebody must need to bet.
        if let AgentOutput::NeedBet { options } = alice.auto_respond_if_needed().unwrap() {
            record(
                alice.bet(pick_passive(&options)).unwrap(),
                ALICE_DID,
                &mut transcript,
                &mut for_bob,
            );
        } else if let AgentOutput::NeedBet { options } = bob.auto_respond_if_needed().unwrap() {
            record(
                bob.bet(pick_passive(&options)).unwrap(),
                BOB_DID,
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

    for (i, (author, action)) in transcript.iter().enumerate() {
        let out = watcher.receive_action(action, author).unwrap();
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

    fn broadcast(
        queues: &mut [Vec<(usize, Vec<u8>)>],
        active: &[usize],
        from: usize,
        actions: &[Vec<u8>],
    ) {
        for &i in active {
            if i != from {
                queues[i].extend(actions.iter().map(|a| (from, a.clone())));
            }
        }
    }

    /// Pump messages + scripted bets until every active agent is Complete.
    fn run_hand(
        agents: &mut [PlayerAgent],
        queues: &mut [Vec<(usize, Vec<u8>)>],
        dids: &[&str],
        active: &[usize],
        mut pick: impl FnMut(usize, &[BetAction]) -> BetAction,
    ) {
        let mut bets_made = 0usize;
        for _ in 0..5000 {
            let mut progress = false;
            for &i in active {
                let queue = std::mem::take(&mut queues[i]);
                for (author, action) in &queue {
                    if let AgentOutput::Actions(out) =
                        agents[i].receive_action(action, dids[*author]).unwrap()
                    {
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
    let mut queues: Vec<Vec<(usize, Vec<u8>)>> = vec![vec![]; 3];
    let everyone = [0usize, 1, 2];
    for i in 0..3 {
        let out = agents[i].receive_table(&table_cbor).unwrap();
        if let AgentOutput::Actions(a) = out {
            broadcast(&mut queues, &everyone, i, &a);
        }
    }
    run_hand(
        &mut agents,
        &mut queues,
        &dids,
        &everyone,
        |n, options| match n {
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
        },
    );

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
    run_hand(&mut agents, &mut queues, &dids, &survivors, |_, options| {
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

/// Regression for the resume/replay consensus failure (table 3mrqkfazprl2e,
/// 2026-07-28): received bets used to be attributed to whoever the state
/// machine thought was next to act, not to the record's author. A bet record
/// fed with the wrong author must be rejected, never silently applied as the
/// in-turn player's action.
#[test]
fn bet_records_are_attributed_to_their_author() {
    let mut alice = PlayerAgent::new(ALICE_DID, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(BOB_DID, b"bob_seed").unwrap();
    let table_cbor = make_table_cbor(&[ALICE_DID, BOB_DID], 1000, 10);

    let alice_commit = unwrap_actions(alice.receive_table(&table_cbor).unwrap());
    let bob_commit = unwrap_actions(bob.receive_table(&table_cbor).unwrap());
    let mut for_bob = Vec::new();
    let mut for_alice = Vec::new();
    collect_output(
        alice.receive_action(&bob_commit[0], BOB_DID).unwrap(),
        &mut for_bob,
    );
    collect_output(
        bob.receive_action(&alice_commit[0], ALICE_DID).unwrap(),
        &mut for_alice,
    );

    // Relay through shuffle/lock/deal until someone is prompted to bet.
    // Heads-up preflop: alice (button/SB) acts first.
    let (a_bet, b_bet) = relay_until_stuck_with_queues(&mut alice, &mut bob, for_bob, for_alice);
    assert!(a_bet.is_some(), "alice should be first to act heads-up");
    assert!(b_bet.is_none());

    // Alice emits her bet.
    let emitted = unwrap_actions(alice.bet(BetAction::Call).unwrap());
    let bet_cbor = &emitted[0];

    // Feeding it to bob with the WRONG author (claiming bob wrote it) must
    // fail — it is alice's turn, and attribution must come from the author,
    // not from whose turn the receiver thinks it is.
    let err = bob.receive_action(bet_cbor, BOB_DID);
    assert!(
        err.is_err(),
        "misattributed bet must be rejected, got {:?}",
        err
    );

    // An author who isn't at the table at all is rejected too.
    assert!(bob.receive_action(bet_cbor, "did:plc:mallory").is_err());

    // With the true author it applies fine.
    bob.receive_action(bet_cbor, ALICE_DID).unwrap();
}

/// Replay must be order-independent: per-author order is fixed (a PDS repo
/// replays in seq order) but the interleaving across authors is arbitrary —
/// exactly what the firehose backfill does on a page reload. With
/// retry-buffering (like the web client's pending queue), every interleaving
/// must converge to the same final state. Pre-fix, an early-arriving bet was
/// silently applied as the wrong player's action and the replayed state
/// diverged — the client then charged ahead publishing actions for a game
/// nobody else was playing.
#[test]
fn replay_converges_from_any_author_interleaving() {
    let mut alice = PlayerAgent::new(ALICE_DID, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(BOB_DID, b"bob_seed").unwrap();
    let table_cbor = make_table_cbor(&[ALICE_DID, BOB_DID], 1000, 10);

    // Play a hand with a raise in it, recording (author, action) in order.
    let mut transcript: Vec<(&str, Vec<u8>)> = Vec::new();
    let mut for_alice: Vec<Vec<u8>> = Vec::new();
    let mut for_bob: Vec<Vec<u8>> = Vec::new();
    let mut bets_made = 0usize;

    let record = |out: AgentOutput,
                  author: &'static str,
                  transcript: &mut Vec<(&str, Vec<u8>)>,
                  inbox: &mut Vec<Vec<u8>>| {
        if let AgentOutput::Actions(actions) = out {
            for a in actions {
                transcript.push((author, a.clone()));
                inbox.push(a);
            }
        }
    };

    record(
        alice.receive_table(&table_cbor).unwrap(),
        ALICE_DID,
        &mut transcript,
        &mut for_bob,
    );
    record(
        bob.receive_table(&table_cbor).unwrap(),
        BOB_DID,
        &mut transcript,
        &mut for_alice,
    );

    for _ in 0..2000 {
        if matches!(alice.phase(), Phase::Complete) && matches!(bob.phase(), Phase::Complete) {
            break;
        }
        if let Some(action) = (!for_alice.is_empty()).then(|| for_alice.remove(0)) {
            record(
                alice.receive_action(&action, BOB_DID).unwrap(),
                ALICE_DID,
                &mut transcript,
                &mut for_bob,
            );
            continue;
        }
        if let Some(action) = (!for_bob.is_empty()).then(|| for_bob.remove(0)) {
            record(
                bob.receive_action(&action, ALICE_DID).unwrap(),
                BOB_DID,
                &mut transcript,
                &mut for_alice,
            );
            continue;
        }
        // The first bet is a raise so the replay has turn-order to get wrong.
        let mut spice = |options: &[BetAction]| {
            bets_made += 1;
            if bets_made == 1 {
                BetAction::Raise(40)
            } else {
                pick_passive(options)
            }
        };
        if let AgentOutput::NeedBet { options } = alice.auto_respond_if_needed().unwrap() {
            record(
                alice.bet(spice(&options)).unwrap(),
                ALICE_DID,
                &mut transcript,
                &mut for_bob,
            );
        } else if let AgentOutput::NeedBet { options } = bob.auto_respond_if_needed().unwrap() {
            record(
                bob.bet(spice(&options)).unwrap(),
                BOB_DID,
                &mut transcript,
                &mut for_alice,
            );
        }
    }
    assert!(matches!(alice.phase(), Phase::Complete));
    assert!(bets_made > 2, "the hand should have had a betting sequence");

    // Reference: an in-order replay.
    let reference = {
        let mut w = PlayerAgent::new("did:plc:watcher", b"w_seed").unwrap();
        w.receive_table(&table_cbor).unwrap();
        for (author, action) in &transcript {
            w.receive_action(action, author).unwrap();
        }
        assert!(matches!(w.phase(), Phase::Complete));
        w.game_state_json()
    };

    // Adversarial replays: per-author queues, interleaved by a seeded LCG,
    // with rejected actions retried after the next successful apply (the web
    // client's pending-buffer semantics).
    for seed in 0u64..8 {
        let mut queues: Vec<std::collections::VecDeque<&Vec<u8>>> = vec![
            transcript
                .iter()
                .filter(|(a, _)| *a == ALICE_DID)
                .map(|(_, c)| c)
                .collect(),
            transcript
                .iter()
                .filter(|(a, _)| *a == BOB_DID)
                .map(|(_, c)| c)
                .collect(),
        ];
        let authors = [ALICE_DID, BOB_DID];
        let mut w = PlayerAgent::new("did:plc:watcher", b"w_seed").unwrap();
        w.receive_table(&table_cbor).unwrap();

        let mut lcg = seed
            .wrapping_mul(2862933555777941757)
            .wrapping_add(3037000493);
        let mut stalled = 0;
        while queues.iter().any(|q| !q.is_empty()) {
            lcg = lcg
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let first = (lcg >> 33) as usize % 2;
            let mut progressed = false;
            for &i in &[first, 1 - first] {
                if let Some(action) = queues[i].front() {
                    if w.receive_action(action, authors[i]).is_ok() {
                        queues[i].pop_front();
                        progressed = true;
                        break;
                    }
                }
            }
            if progressed {
                stalled = 0;
            } else {
                stalled += 1;
                assert!(
                    stalled < 4,
                    "replay deadlocked (seed {}): neither author's next action applies; \
                     remaining alice={} bob={}",
                    seed,
                    queues[0].len(),
                    queues[1].len()
                );
            }
        }
        assert!(
            matches!(w.phase(), Phase::Complete),
            "seed {}: replay did not complete",
            seed
        );
        assert_eq!(
            w.game_state_json(),
            reference,
            "seed {}: replayed state diverged from the in-order reference",
            seed
        );
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
    let _ = bob.receive_action(&alice_commit[0], ALICE_DID).unwrap();

    let waiting: serde_json::Value = serde_json::from_str(&bob.waiting_on_json()).unwrap();
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    assert_eq!(waiting[0]["kind"], "shuffleDeck");
    assert_eq!(waiting[0]["seats"], serde_json::json!([0]));
}
