//! Test the simulator / dummy bus.

use cardcore_poker::sim::{BotStrategy, GameEvent, SimConfig, Simulator};

#[test]
fn simulate_2_player_passive() {
    let config = SimConfig {
        num_players: 2,
        starting_chips: 1000,
        small_blind: 10,
        strategy: BotStrategy::Passive,
        rng_seed: 42,
    };
    let mut sim = Simulator::new(config).unwrap();
    sim.run().unwrap();

    let events = sim.events();
    eprintln!("Events ({}):", events.len());
    for e in events {
        eprintln!("  {:?}", e);
    }

    // Should have basic events
    assert!(
        events
            .iter()
            .any(|e| matches!(e, GameEvent::TableCreated { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, GameEvent::HoleCardsDealt { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, GameEvent::CommunityDealt { street, .. } if street == "flop"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, GameEvent::CommunityDealt { street, .. } if street == "river"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, GameEvent::ShowdownReveal { .. }))
    );
    assert!(events.iter().any(|e| matches!(e, GameEvent::SeedsVerified)));

    // All players should have hole cards
    let hole_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, GameEvent::HoleCardsDealt { .. }))
        .collect();
    assert_eq!(hole_events.len(), 2);
}

#[test]
fn simulate_3_player_passive() {
    let config = SimConfig {
        num_players: 3,
        starting_chips: 1000,
        small_blind: 10,
        strategy: BotStrategy::Passive,
        rng_seed: 99,
    };
    let mut sim = Simulator::new(config).unwrap();
    sim.run().unwrap();

    let events = sim.events();
    let hole_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, GameEvent::HoleCardsDealt { .. }))
        .collect();
    assert_eq!(hole_events.len(), 3);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, GameEvent::CommunityDealt { street, .. } if street == "flop"))
    );
}

#[test]
fn simulate_random_strategy() {
    // Run a few random games to make sure they complete
    for seed in 0..10 {
        let config = SimConfig {
            num_players: 2,
            starting_chips: 1000,
            small_blind: 10,
            strategy: BotStrategy::Random,
            rng_seed: seed,
        };
        let mut sim = Simulator::new(config).unwrap();
        sim.run().unwrap();
        // Seeds should always be verified, even if the game ended by fold
        assert!(
            sim.events()
                .iter()
                .any(|e| matches!(e, GameEvent::SeedsVerified)),
            "seed={}: seeds should always be verified",
            seed
        );
    }
}

#[test]
fn events_serialize_to_json() {
    let config = SimConfig::default();
    let mut sim = Simulator::new(config).unwrap();
    sim.run().unwrap();

    let json = serde_json::to_string_pretty(sim.events()).unwrap();
    assert!(json.contains("tableCreated"));
    assert!(json.contains("holeCardsDealt"));
    eprintln!(
        "JSON output ({} bytes):\n{}",
        json.len(),
        &json[..json.len().min(2000)]
    );
}
