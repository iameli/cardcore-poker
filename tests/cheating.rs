//! Cheating scenarios: forged action records pushed to the expected rkeys of
//! a player's repo. The engine must classify every received action as either
//! OutOfOrder (not valid YET — concurrent replay reorders honestly-authored
//! records, so these buffer and retry) or a terminal violation (can NEVER be
//! valid — the author is cheating or their client is broken). The client UI
//! surfaces the latter; mixing the two up either stalls honest games or lets
//! forged actions hide in the retry buffer.

use cardcore_poker::Error;
use cardcore_poker::agent::{AgentOutput, PlayerAgent};
use cardcore_poker::lexicon::re_cardco::poker::action::ActionAction;
use cardcore_poker::lexicon::re_cardco::poker::table::Table as LexTable;
use cardcore_poker::lexicon::re_cardco::poker::*;
use jacquard_common::deps::bytes::Bytes;
use jacquard_common::types::string::Datetime;

const ALICE_DID: &str = "did:plc:alice";
const BOB_DID: &str = "did:plc:bob";
const CAROL_DID: &str = "did:plc:carol";

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

fn forge(action: &ActionAction) -> Vec<u8> {
    dasl::drisl::to_vec(action).unwrap()
}

fn forge_bet(action: BetAction2, amount: Option<i64>) -> Vec<u8> {
    forge(&ActionAction::Bet(Box::new(Bet {
        action,
        amount,
        extra_data: None,
    })))
}

// Alias mirroring src/agent.rs — the lexicon's BetAction, not the game's.
use cardcore_poker::lexicon::re_cardco::poker::BetAction as BetAction2;

fn collect_output(output: AgentOutput, queue: &mut Vec<Vec<u8>>) {
    if let AgentOutput::Actions(actions) = output {
        queue.extend(actions);
    }
}

/// Relay a two-player table from receive_table until someone is prompted to
/// bet. Returns which agent was prompted (0 = alice, 1 = bob).
fn relay_to_first_bet(alice: &mut PlayerAgent, bob: &mut PlayerAgent) -> usize {
    let table_cbor = make_table_cbor(&[ALICE_DID, BOB_DID], 1000, 10);
    let alice_commit = match alice.receive_table(&table_cbor).unwrap() {
        AgentOutput::Actions(a) => a,
        other => panic!("expected commit, got {:?}", other),
    };
    let bob_commit = match bob.receive_table(&table_cbor).unwrap() {
        AgentOutput::Actions(a) => a,
        other => panic!("expected commit, got {:?}", other),
    };
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

    for _ in 0..1000 {
        if for_bob.is_empty() && for_alice.is_empty() {
            match alice.auto_respond_if_needed().unwrap() {
                AgentOutput::NeedBet { .. } => return 0,
                AgentOutput::Actions(a) => {
                    for_bob.extend(a);
                    continue;
                }
                AgentOutput::Waiting => {}
            }
            match bob.auto_respond_if_needed().unwrap() {
                AgentOutput::NeedBet { .. } => return 1,
                AgentOutput::Actions(a) => {
                    for_alice.extend(a);
                    continue;
                }
                AgentOutput::Waiting => panic!("relay stalled before any bet"),
            }
        }
        if let Some(action) = (!for_bob.is_empty()).then(|| for_bob.remove(0)) {
            collect_output(
                bob.receive_action(&action, ALICE_DID).unwrap(),
                &mut for_alice,
            );
        }
        if let Some(action) = (!for_alice.is_empty()).then(|| for_alice.remove(0)) {
            collect_output(
                alice.receive_action(&action, BOB_DID).unwrap(),
                &mut for_bob,
            );
        }
    }
    panic!("relay exceeded max iterations");
}

/// A forged under-raise at the acting player's expected rkey is a terminal
/// violation — never something to buffer and retry.
#[test]
fn forged_under_raise_is_a_violation() {
    let mut alice = PlayerAgent::new(ALICE_DID, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(BOB_DID, b"bob_seed").unwrap();
    let who = relay_to_first_bet(&mut alice, &mut bob);
    assert_eq!(who, 0, "heads-up: alice (button/SB) acts first");

    // Blinds are 10/20 — a raise to a TOTAL of 5 is below both the current
    // bet and the minimum raise. It's alice's turn, so the bet is evaluated
    // (not out of order) and must be condemned.
    let cheat = forge_bet(BetAction2::Other("raise".into()), Some(5));
    let err = bob.receive_action(&cheat, ALICE_DID).unwrap_err();
    assert!(
        matches!(err, Error::InvalidAction(_)),
        "under-raise must be a terminal violation, got: {:?}",
        err
    );

    // The same forged bet delivered as BOB's (whose turn it is NOT) is merely
    // out of order — during replay, honest bets routinely arrive early.
    let err = bob.receive_action(&cheat, BOB_DID).unwrap_err();
    assert!(
        matches!(err, Error::OutOfOrder(_)),
        "out-of-turn bets must buffer, got: {:?}",
        err
    );
}

/// A bet from a player who isn't even at the table can never apply.
#[test]
fn action_from_unknown_author_is_a_violation() {
    let mut alice = PlayerAgent::new(ALICE_DID, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(BOB_DID, b"bob_seed").unwrap();
    relay_to_first_bet(&mut alice, &mut bob);

    let cheat = forge_bet(BetAction2::Call, None);
    let err = bob.receive_action(&cheat, "did:plc:mallory").unwrap_err();
    assert!(
        matches!(err, Error::InvalidAction(_)),
        "unknown author must be a terminal violation, got: {:?}",
        err
    );
}

/// Checking while facing a bet is a violation once it's your turn.
#[test]
fn forged_check_facing_a_bet_is_a_violation() {
    let mut alice = PlayerAgent::new(ALICE_DID, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(BOB_DID, b"bob_seed").unwrap();
    let who = relay_to_first_bet(&mut alice, &mut bob);
    assert_eq!(who, 0);

    // Alice is the SB facing the BB — a "check" dodges paying the difference.
    let cheat = forge_bet(BetAction2::Check, None);
    let err = bob.receive_action(&cheat, ALICE_DID).unwrap_err();
    assert!(
        matches!(err, Error::InvalidAction(_)),
        "check-facing-a-bet must be a terminal violation, got: {:?}",
        err
    );
}

/// A second commitSeed from a player who already committed is a violation —
/// re-committing after seeing more information is exactly the fraud the
/// commitment scheme exists to prevent. (The firehose dedups (did, seq), so a
/// duplicate can only arrive as a NEW record the author deliberately wrote.)
#[test]
fn duplicate_commit_seed_is_a_violation() {
    let dids = [ALICE_DID, BOB_DID, CAROL_DID];
    let table_cbor = make_table_cbor(&dids, 1000, 10);
    let mut alice = PlayerAgent::new(ALICE_DID, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(BOB_DID, b"bob_seed").unwrap();

    let alice_commit = match alice.receive_table(&table_cbor).unwrap() {
        AgentOutput::Actions(a) => a,
        other => panic!("expected commit, got {:?}", other),
    };
    let _ = bob.receive_table(&table_cbor).unwrap();

    // Carol hasn't committed yet, so the phase is still CommitSeeds when
    // alice's second commit lands.
    bob.receive_action(&alice_commit[0], ALICE_DID).unwrap();
    let err = bob.receive_action(&alice_commit[0], ALICE_DID).unwrap_err();
    assert!(
        matches!(err, Error::InvalidAction(_)),
        "duplicate commit must be a terminal violation, got: {:?}",
        err
    );
}

/// A shuffled deck with a missing card can never be valid — a 51-card deck
/// means a card was palmed.
#[test]
fn short_deck_shuffle_is_a_violation() {
    let table_cbor = make_table_cbor(&[ALICE_DID, BOB_DID], 1000, 10);
    let mut alice = PlayerAgent::new(ALICE_DID, b"alice_seed").unwrap();
    let mut bob = PlayerAgent::new(BOB_DID, b"bob_seed").unwrap();
    let alice_commit = match alice.receive_table(&table_cbor).unwrap() {
        AgentOutput::Actions(a) => a,
        other => panic!("expected commit, got {:?}", other),
    };
    let _ = bob.receive_table(&table_cbor).unwrap();
    // Both commits in → phase Shuffle{next: alice}.
    bob.receive_action(&alice_commit[0], ALICE_DID).unwrap();

    let cheat = forge(&ActionAction::ShuffleDeck(Box::new(ShuffleDeck {
        deck: vec![Bytes::from(vec![1u8; 32]); 51],
        extra_data: None,
    })));
    let err = bob.receive_action(&cheat, ALICE_DID).unwrap_err();
    assert!(
        matches!(err, Error::InvalidAction(_)),
        "51-card shuffle must be a terminal violation, got: {:?}",
        err
    );

    // The same deck authored by bob (not the shuffler) is out of order, not
    // (yet) evidence of cheating — order gates first, then content.
    let ok_size = forge(&ActionAction::ShuffleDeck(Box::new(ShuffleDeck {
        deck: vec![Bytes::from(vec![1u8; 32]); 52],
        extra_data: None,
    })));
    let err = bob.receive_action(&ok_size, BOB_DID).unwrap_err();
    assert!(
        matches!(err, Error::OutOfOrder(_)),
        "out-of-turn shuffle must buffer, got: {:?}",
        err
    );
}

/// A bet published while the table is still committing/shuffling is out of
/// order (during replay a bet from a later phase can arrive early) — it must
/// buffer, not be condemned.
#[test]
fn early_bet_buffers_instead_of_violating() {
    let table_cbor = make_table_cbor(&[ALICE_DID, BOB_DID], 1000, 10);
    let mut bob = PlayerAgent::new(BOB_DID, b"bob_seed").unwrap();
    let _ = bob.receive_table(&table_cbor).unwrap();

    let early = forge_bet(BetAction2::Call, None);
    let err = bob.receive_action(&early, ALICE_DID).unwrap_err();
    assert!(
        matches!(err, Error::OutOfOrder(_)),
        "early bet must buffer, got: {:?}",
        err
    );
}
