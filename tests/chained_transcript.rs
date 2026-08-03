//! Chained (settlement) driving: proves that N independent agents build ONE
//! bounded, contiguous, globally ordered action chain that ends with every
//! seat's `VerifySeed` — the shape the atbloons `CARDCORE_POKER_V1` transcript
//! evaluator accepts for a settlement-valid paid hand.
//!
//! The canonical global order is `state.valid_actions()[0]`, exactly what the
//! offline fixture generator (`tools/cardcore-fixtures`) uses and the atbloons
//! evaluator already validates. Here we drive that order in a decentralized
//! way: each agent is a separate `PlayerAgent` (its own seed, its own repo),
//! they exchange only published action CBOR, and we assert they independently
//! agree on the same next action at every step. That agreement is what lets
//! real, separate players build a single chain with no forks.

use cardcore_poker::agent::PlayerAgent;
use cardcore_poker::lexicon::re_cardco::poker::table::Table as LexTable;
use jacquard_common::types::string::Datetime;
use serde_json::Value;

/// One record in the produced global chain.
struct ChainLink {
    seat: usize,
    kind: String,
    cbor: Vec<u8>,
}

/// A scripted betting choice for a seat, given the JSON `options` array from
/// `next_action`. Returns the produce_bet string (e.g. "check", "call").
type BetBot = fn(seat: usize, options: &[Value]) -> String;

fn passive(_seat: usize, options: &[Value]) -> String {
    if options.iter().any(|o| o == "Check") {
        "check".into()
    } else if options.iter().any(|o| o == "Call") {
        "call".into()
    } else {
        "fold".into()
    }
}

/// Folds at the first point a seat faces a bet it cannot check (heads-up, the
/// small blind acts first preflop and must call the big blind — so it folds),
/// producing a win-by-fold hand that still needs the mandatory seed reveals.
fn facing_bet_folds(seat: usize, options: &[Value]) -> String {
    if options.iter().any(|o| o == "Fold") && !options.iter().any(|o| o == "Check") {
        "fold".into()
    } else {
        passive(seat, options)
    }
}

fn table_cbor(dids: &[String], starting_chips: u64, small_blind: u64) -> Vec<u8> {
    let table = LexTable {
        players: dids.iter().map(|d| d.clone().into()).collect(),
        starting_chips: starting_chips as i64,
        small_blind: small_blind as i64,
        created_at: Datetime::now(),
        extra_data: None,
    };
    dasl::drisl::to_vec(&table).expect("encode table")
}

/// Drive one hand across `seats` chained agents and return the produced chain.
fn drive_hand(seats: usize, starting_chips: u64, small_blind: u64, bot: BetBot) -> Vec<ChainLink> {
    let dids: Vec<String> = (0..seats).map(|s| format!("did:plc:chained{s}")).collect();
    let mut agents: Vec<PlayerAgent> = dids
        .iter()
        .enumerate()
        .map(|(seat, did)| {
            let seed = format!("chained-seed-{seat}").into_bytes();
            let mut agent = PlayerAgent::new(did, &seed).expect("agent");
            agent.set_chained(true);
            agent
        })
        .collect();

    let table = table_cbor(&dids, starting_chips, small_blind);
    for agent in &mut agents {
        let out = agent.receive_table(&table).expect("receive table");
        // Chained agents never auto-emit; the table only advances phase.
        assert_eq!(agent_output_kind(out), "waiting", "chained agents must not emit on table");
    }

    let mut chain: Vec<ChainLink> = Vec::new();
    for _ in 0..5_000 {
        // Every agent must independently agree on the same next action. This is
        // the decentralization property: separate repos, no coordinator, one
        // chain. Compare the canonical part (seat, kind, options) across agents;
        // the `mine` flag is intentionally agent-relative and excluded.
        let next0 = agents[0].next_action_json();
        let canon0 = canonical(&next0);
        for (seat, agent) in agents.iter().enumerate() {
            assert_eq!(
                canonical(&agent.next_action_json()),
                canon0,
                "seat {seat} disagreed on the next action",
            );
        }

        if next0 == "null" {
            // Chain complete only in the Complete phase with all seeds verified.
            assert_eq!(format!("{:?}", agents[0].phase()), "Complete", "null next before Complete");
            let chain_links = chain.len();
            assert!(chain_links > 0, "empty chain");
            return chain;
        }

        let next: Value = serde_json::from_str(&next0).expect("next_action json");
        let seat = next["seat"].as_u64().expect("seat") as usize;
        let kind = next["kind"].as_str().expect("kind").to_string();

        let cbor = if kind == "bet" {
            let empty = Vec::new();
            let options = next["options"].as_array().unwrap_or(&empty);
            let choice = bot(seat, options);
            agents[seat].produce_bet(parse_bet(&choice)).expect("produce bet")
        } else {
            let produced = agents[seat].produce_next().expect("produce next");
            assert!(!produced.is_empty(), "seat {seat} produced nothing for {kind}");
            produced
        };

        // Publish → every agent (author included) applies the same record in
        // chain order. This is exactly what the firehose delivers live.
        for agent in &mut agents {
            agent.receive_action(&cbor, &dids[seat]).expect("receive action");
        }
        chain.push(ChainLink { seat, kind, cbor });
    }
    panic!("hand did not complete within the action bound");
}

/// The canonical (agent-independent) view of a `next_action` JSON: the same
/// object with the agent-relative `mine` flag removed. All agents must agree
/// on this.
fn canonical(next_action_json: &str) -> String {
    if next_action_json == "null" {
        return "null".to_string();
    }
    let mut value: Value = serde_json::from_str(next_action_json).expect("next_action json");
    if let Some(object) = value.as_object_mut() {
        object.remove("mine");
    }
    serde_json::to_string(&value).expect("canonical json")
}

fn parse_bet(choice: &str) -> cardcore_poker::game::BetAction {
    use cardcore_poker::game::BetAction;
    match choice {
        "fold" => BetAction::Fold,
        "check" => BetAction::Check,
        "call" => BetAction::Call,
        "allIn" => BetAction::AllIn,
        _ => BetAction::Fold,
    }
}

fn agent_output_kind(output: cardcore_poker::agent::AgentOutput) -> &'static str {
    use cardcore_poker::agent::AgentOutput;
    match output {
        AgentOutput::Actions(_) => "actions",
        AgentOutput::NeedBet { .. } => "need_bet",
        AgentOutput::Waiting => "waiting",
    }
}

/// Shared end-state assertions for a settlement-valid chain.
fn assert_settlement_valid(seats: usize, chain: &[ChainLink]) {
    // Contiguous global sequence: the chain is one list; its indices ARE the
    // seq values the records carry (seq = position). Verify the tail is exactly
    // one VerifySeed per seat — the mandatory final seed-reveal sequence.
    let verify_count = chain.iter().filter(|l| l.kind == "verifySeed").count();
    assert_eq!(verify_count, seats, "every seat must reveal its seed");

    let tail = &chain[chain.len() - seats..];
    assert!(
        tail.iter().all(|l| l.kind == "verifySeed"),
        "seed reveals must be the final actions in the chain",
    );
    let revealing: std::collections::BTreeSet<usize> = tail.iter().map(|l| l.seat).collect();
    assert_eq!(revealing.len(), seats, "each seat reveals exactly once");

    // The first action is always seat-ordered commit seeds.
    assert_eq!(chain[0].kind, "commitSeed", "chain must open with commit seeds");

    // A fresh observer (the node/evaluator's perspective) replays the whole
    // chain in order and must also reach Complete with all seeds verified.
    let dids: Vec<String> = (0..seats).map(|s| format!("did:plc:chained{s}")).collect();
    let mut observer = PlayerAgent::new("did:plc:observer", b"observer-seed").expect("observer");
    observer.set_chained(true);
    // Rebuild the table exactly as the agents saw it is unnecessary for replay
    // attribution; the observer just needs the roster. Feed a matching table.
    let table = table_cbor(&dids, 100, 5);
    observer.receive_table(&table).expect("observer table");
    for link in chain {
        observer
            .receive_action(&link.cbor, &dids[link.seat])
            .expect("observer replay");
    }
    assert_eq!(format!("{:?}", observer.phase()), "Complete", "observer replay incomplete");
    assert_eq!(observer.next_action_json(), "null", "observer has pending actions");
}

#[test]
fn two_player_passive_showdown_builds_settlement_valid_chain() {
    let chain = drive_hand(2, 100, 5, passive);
    assert_settlement_valid(2, &chain);
}

#[test]
fn three_player_passive_showdown_builds_settlement_valid_chain() {
    let chain = drive_hand(3, 100, 5, passive);
    assert_settlement_valid(3, &chain);
}

#[test]
fn two_player_fold_builds_settlement_valid_chain() {
    let chain = drive_hand(2, 100, 5, facing_bet_folds);
    // A fold still requires the mandatory seed-reveal tail to settle.
    assert_settlement_valid(2, &chain);
}

#[test]
fn chain_is_bounded() {
    let chain = drive_hand(2, 100, 5, passive);
    assert!(chain.len() < 1_024, "chain must stay within the evaluator bound");
}
