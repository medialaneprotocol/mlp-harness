//! The link model's own properties.
//!
//! The link is a *network*: a rate shaped by a token bucket and a queue, a
//! propagation delay with a jitter distribution, four loss models, a
//! reordering window, a duplication rate, an MTU, scheduled outages, two
//! separate directions, and a carrier handover that costs what a real one
//! costs. These are the properties the model claims, checked.

use mlp_harness::link::{
    Direction, DropReason, Handover, Jitter, Link, LinkProfile, LossModel, OfferOutcome,
    OutageKind, OutageSchedule, OutageWindow, Tier,
};

fn perfect() -> LinkProfile {
    LinkProfile::perfect()
}

#[test]
fn a_perfect_link_delivers_everything_in_order() {
    let mut link = Link::new(perfect());
    for index in 0..10u8 {
        link.offer(Direction::Down, vec![index; 64]);
    }
    let deliveries = link.advance_to(1_000_000);
    assert_eq!(deliveries.len(), 10);
    for (index, delivery) in deliveries.iter().enumerate() {
        assert_eq!(delivery.bytes[0], index as u8);
    }
    assert_eq!(link.stats().dropped(), 0);
}

#[test]
fn the_token_bucket_shapes_a_burst() {
    let mut profile = perfect();
    profile.down.rate_bps = 8_000; // 1 000 bytes per second
    profile.down.burst_bytes = 1_000;
    profile.down.queue_cap_bytes = 10_000;
    profile.down.propagation_us = 0;
    profile.down.jitter = Jitter::none();
    let mut link = Link::new(profile);
    for index in 0..10u8 {
        link.offer(Direction::Down, vec![index; 1_000]);
    }
    let deliveries = link.advance_to(20_000_000);
    assert_eq!(deliveries.len(), 10);
    // The first passes on the bucket's initial tokens; the last waits for
    // nine seconds of rate.
    assert_eq!(deliveries[0].at_us, 0);
    assert!(
        deliveries[9].at_us >= 9_000_000,
        "the last left at {}",
        deliveries[9].at_us
    );
}

#[test]
fn a_full_queue_drops_the_object() {
    let mut profile = perfect();
    profile.down.rate_bps = 8_000;
    profile.down.burst_bytes = 1_000;
    profile.down.queue_cap_bytes = 2_000;
    let mut link = Link::new(profile);
    link.offer(Direction::Down, vec![0; 1_000]);
    link.offer(Direction::Down, vec![1; 1_000]);
    let outcome = link.offer(Direction::Down, vec![2; 1_000]);
    assert_eq!(
        outcome,
        OfferOutcome::Dropped {
            seq: 3,
            reason: DropReason::QueueOverflow
        }
    );
}

#[test]
fn an_oversize_object_is_dropped_not_fragmented() {
    let mut profile = perfect();
    profile.mtu_bytes = 100;
    let mut link = Link::new(profile);
    let outcome = link.offer(Direction::Down, vec![0; 101]);
    assert_eq!(
        outcome,
        OfferOutcome::Dropped {
            seq: 1,
            reason: DropReason::TooLarge
        }
    );
}

#[test]
fn the_two_directions_are_separate_links() {
    let mut profile = perfect();
    profile.down.rate_bps = 8_000;
    profile.down.burst_bytes = 1_000;
    profile.up.rate_bps = 100_000_000;
    profile.up.burst_bytes = 12_500_000;
    let mut link = Link::new(profile);
    // The downlink is slow: two objects fill its queue.
    link.offer(Direction::Down, vec![0; 1_000]);
    link.offer(Direction::Down, vec![1; 1_000]);
    // The uplink is fast: it takes them all.
    for _ in 0..10 {
        link.offer(Direction::Up, vec![0; 1_000]);
    }
    let deliveries = link.advance_to(30_000_000);
    let down = deliveries
        .iter()
        .filter(|delivery| delivery.direction == Direction::Down)
        .count();
    let up = deliveries
        .iter()
        .filter(|delivery| delivery.direction == Direction::Up)
        .count();
    assert_eq!(down, 2);
    assert_eq!(up, 10);
}

#[test]
fn a_blackhole_takes_what_it_covers_in_both_directions() {
    let mut profile = perfect();
    profile.outages = OutageSchedule::of(vec![OutageWindow::new(
        100_000,
        100_000,
        OutageKind::Blackhole,
    )]);
    let mut link = Link::new(profile);
    link.advance_to(150_000);
    assert!(matches!(
        link.offer(Direction::Down, vec![0; 8]),
        OfferOutcome::Dropped { .. }
    ));
    assert!(matches!(
        link.offer(Direction::Up, vec![0; 8]),
        OfferOutcome::Dropped { .. }
    ));
    link.advance_to(300_000);
    link.offer(Direction::Down, vec![1; 8]);
    assert_eq!(link.advance_to(400_000).len(), 1);
}

#[test]
fn a_one_way_blackhole_spares_the_other_direction() {
    let mut profile = perfect();
    profile.outages = OutageSchedule::of(vec![OutageWindow::new(
        0,
        1_000_000,
        OutageKind::OneWayBlackhole {
            direction: Direction::Up,
        },
    )]);
    let mut link = Link::new(profile);
    assert!(matches!(
        link.offer(Direction::Up, vec![0; 8]),
        OfferOutcome::Dropped { .. }
    ));
    assert!(matches!(
        link.offer(Direction::Down, vec![0; 8]),
        OfferOutcome::Queued { .. }
    ));
}

#[test]
fn an_object_in_flight_is_lost_by_an_outage_that_starts() {
    let mut profile = perfect();
    profile.down.propagation_us = 1_000_000;
    profile.down.jitter = Jitter::none();
    profile.outages = OutageSchedule::of(vec![OutageWindow::new(
        500_000,
        1_000_000,
        OutageKind::Blackhole,
    )]);
    let mut link = Link::new(profile);
    link.offer(Direction::Down, vec![0; 8]);
    assert!(link.advance_to(2_000_000).is_empty());
    assert_eq!(link.stats().down.dropped_outage, 1);
}

#[test]
fn a_handover_splits_the_objects_in_flight_and_loses_some() {
    let mut profile = perfect();
    profile.down.propagation_us = 1_000_000;
    profile.down.jitter = Jitter::none();
    let mut link = Link::new(profile.clone());
    for _ in 0..10 {
        link.offer(Direction::Down, vec![0; 8]);
    }
    link.advance_to(1_000);
    assert_eq!(link.in_flight(), 10);
    let mut next = profile.clone();
    next.down.propagation_us = 1_000;
    link.handover(&Handover {
        profile: next,
        split_permille: 500,
        loss_permille: 200,
    });
    assert_eq!(link.stats().down.dropped_handover, 2);
    assert_eq!(link.in_flight(), 8);
}

#[test]
fn the_loss_models_lose_differently() {
    let mut uniform = perfect();
    uniform.down.loss = LossModel::Uniform { permille: 500 };
    uniform.down.jitter = Jitter::none();
    let mut bursty = perfect();
    bursty.down.loss = LossModel::Bursty { permille: 100 };
    bursty.down.jitter = Jitter::none();
    let mut correlated = perfect();
    correlated.down.loss = LossModel::Correlated {
        base_permille: 0,
        queue_permille: 1_000,
    };
    correlated.down.rate_bps = 8_000;
    correlated.down.burst_bytes = 1_000;
    correlated.down.queue_cap_bytes = 10_000;
    correlated.down.jitter = Jitter::none();

    let lost = |profile: LinkProfile| {
        let mut link = Link::new(profile);
        for _ in 0..200 {
            link.offer(Direction::Down, vec![0; 1_000]);
        }
        link.advance_to(60_000_000);
        link.stats().down.dropped_loss
    };

    // The uniform model loses about half.
    let uniform_lost = lost(uniform);
    assert!(
        (80..=120).contains(&uniform_lost),
        "500‰ lost {uniform_lost}/200"
    );
    // The bursty model loses more than its entry probability, because a run
    // takes several objects with it.
    let bursty_lost = lost(bursty);
    assert!(bursty_lost > 50, "the bursty model lost {bursty_lost}/200");
    // The correlated model loses everything once the queue is full.
    let correlated_lost = lost(correlated);
    assert!(
        correlated_lost > 100,
        "the correlated model lost {correlated_lost}/200"
    );
}

#[test]
fn the_same_profile_gives_the_same_run() {
    let mut profile = LinkProfile::tier(Tier::T2);
    profile.down.reorder_permille = 100;
    profile.down.duplicate_permille = 50;
    let run = || {
        let mut link = Link::new(profile.clone());
        for index in 0..50u8 {
            link.offer(Direction::Down, vec![index; 32]);
        }
        let deliveries = link.advance_to(30_000_000);
        (
            deliveries
                .iter()
                .map(|delivery| (delivery.seq, delivery.at_us, delivery.duplicate))
                .collect::<Vec<_>>(),
            link.stats(),
            link.events().to_vec(),
        )
    };
    assert_eq!(run().0, run().0);
    assert_eq!(run().1, run().1);
    assert_eq!(run().2, run().2);
}

#[test]
fn a_duplicated_object_is_delivered_twice() {
    let mut profile = perfect();
    profile.down.duplicate_permille = 1_000;
    let mut link = Link::new(profile);
    link.offer(Direction::Down, vec![0; 8]);
    let deliveries = link.advance_to(1_000_000);
    assert_eq!(deliveries.len(), 2);
    assert!(deliveries.iter().any(|delivery| delivery.duplicate));
    assert!(deliveries.iter().any(|delivery| !delivery.duplicate));
}
