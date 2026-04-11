//! Test DAG-CBOR serialization of lexicon types via dasl.

use cardcore_poker::lexicon::re_cardco::poker::*;
use cardcore_poker::lexicon::re_cardco::poker::table::*;
use jacquard_common::deps::bytes::Bytes;
use jacquard_common::types::string::Datetime;

#[test]
fn table_cbor_roundtrip() {
    let table = Table {
        players: vec!["did:plc:alice".to_string().into(), "did:plc:bob".to_string().into()],
        starting_chips: 1000,
        small_blind: 10,
        created_at: Datetime::now(),
        extra_data: None,
    };

    let cbor = dasl::drisl::to_vec(&table).unwrap();
    assert!(!cbor.is_empty());

    let decoded: Table = dasl::drisl::from_slice(&cbor).unwrap();
    assert_eq!(decoded.starting_chips, 1000);
    assert_eq!(decoded.small_blind, 10);
    assert_eq!(decoded.players.len(), 2);
}

#[test]
fn action_commit_seed_cbor_roundtrip() {
    let commitment = Bytes::from(vec![0xABu8; 32]);
    let commit = CommitSeed {
        commitment: commitment.clone(),
        extra_data: None,
    };

    let cbor = dasl::drisl::to_vec(&commit).unwrap();
    let decoded: CommitSeed = dasl::drisl::from_slice(&cbor).unwrap();
    assert_eq!(decoded.commitment, commitment);
}

#[test]
fn action_bet_cbor_roundtrip() {
    let bet = Bet {
        action: BetAction::Call,
        amount: None,
        extra_data: None,
    };

    let cbor = dasl::drisl::to_vec(&bet).unwrap();
    let decoded: Bet = dasl::drisl::from_slice(&cbor).unwrap();
    assert_eq!(decoded.action, BetAction::Call);
    assert_eq!(decoded.amount, None);

    // Raise with amount
    let raise = Bet {
        action: BetAction::Other("raise".into()),
        amount: Some(200),
        extra_data: None,
    };

    let cbor = dasl::drisl::to_vec(&raise).unwrap();
    let decoded: Bet = dasl::drisl::from_slice(&cbor).unwrap();
    assert_eq!(decoded.amount, Some(200));
}

#[test]
fn action_reveal_lock_key_cbor_roundtrip() {
    let reveal = RevealLockKey {
        deck_position: 7,
        scalar: Bytes::from(vec![0x42u8; 32]),
        extra_data: None,
    };

    let cbor = dasl::drisl::to_vec(&reveal).unwrap();
    let decoded: RevealLockKey = dasl::drisl::from_slice(&cbor).unwrap();
    assert_eq!(decoded.deck_position, 7);
    assert_eq!(decoded.scalar.len(), 32);
}

#[test]
fn action_reveal_hand_cbor_roundtrip() {
    let reveal = RevealHand {
        reveals: vec![
            PositionScalar {
                deck_position: 3,
                scalar: Bytes::from(vec![0x11u8; 32]),
                extra_data: None,
            },
            PositionScalar {
                deck_position: 5,
                scalar: Bytes::from(vec![0x22u8; 32]),
                extra_data: None,
            },
        ],
        extra_data: None,
    };

    let cbor = dasl::drisl::to_vec(&reveal).unwrap();
    let decoded: RevealHand = dasl::drisl::from_slice(&cbor).unwrap();
    assert_eq!(decoded.reveals.len(), 2);
    assert_eq!(decoded.reveals[0].deck_position, 3);
    assert_eq!(decoded.reveals[1].deck_position, 5);
}

#[test]
fn action_shuffle_deck_cbor_roundtrip() {
    let deck: Vec<Bytes> = (0..52).map(|i| Bytes::from(vec![i as u8; 32])).collect();
    let shuffle = ShuffleDeck {
        deck: deck.clone(),
        extra_data: None,
    };

    let cbor = dasl::drisl::to_vec(&shuffle).unwrap();
    let decoded: ShuffleDeck = dasl::drisl::from_slice(&cbor).unwrap();
    assert_eq!(decoded.deck.len(), 52);
    assert_eq!(decoded.deck[0], deck[0]);
    assert_eq!(decoded.deck[51], deck[51]);
}

#[test]
fn action_verify_seed_cbor_roundtrip() {
    let verify = VerifySeed {
        seed: Bytes::from(b"my_secret_seed_value".to_vec()),
        extra_data: None,
    };

    let cbor = dasl::drisl::to_vec(&verify).unwrap();
    let decoded: VerifySeed = dasl::drisl::from_slice(&cbor).unwrap();
    assert_eq!(decoded.seed, verify.seed);
}
