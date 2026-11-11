//! Boundary tests for `stele_site::terms`.
//!
//! The policy functions panic on violation, so each clamp is exercised on
//! both sides with `catch_unwind` and the panic message is checked to prove
//! which rule fired. The two `*_precedes_*` tests prove the ratchet block runs
//! before the absolute clamps (error order is parity).

use stele_site::{SiteConfig, terms};

/// A configuration that satisfies every clamp with `protocol_bps = 500` and
/// `protocol_rent_bps = 500` (rent total 3_500). Individual tests move one
/// field at a time.
fn config() -> SiteConfig {
    SiteConfig {
        take_bps: 12_000,
        payout_bps: 10_500,
        reversion_bps: 9_000,
        max_reversion_weeks: 12,
        cooldown_secs: 3_600,
        floor_delta_bps: 500,
        floor_change_cooldown_secs: 86_400,
        max_ask_bps: 20_000,
        min_rent_bps: 100,
        site_rent_bps: 3_000,
        max_rental_term_secs: 2_592_000,
    }
}

/// Runs `f` and returns its panic message.
fn panic_message<F: FnOnce()>(f: F) -> String {
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .err()
        .expect("call must panic");
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        String::from("<non-string panic payload>")
    }
}

/// Asserts that `f` panics with a message containing `needle`.
fn assert_panics_with<F: FnOnce()>(f: F, needle: &str) {
    let message = panic_message(f);
    assert!(
        message.contains(needle),
        "panic message {message:?} does not contain {needle:?}"
    );
}

// ---------------------------------------------------------------- economics

#[test]
fn payout_bps_floor_boundary() {
    let mut c = config();

    c.payout_bps = 10_000;
    terms::check_config_absolute(&c);

    c.payout_bps = 9_999;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "payout_bps must be at least 10_000",
    );
}

#[test]
fn take_bps_ceiling_boundary() {
    let mut c = config();

    c.take_bps = 30_000;
    terms::check_config_absolute(&c);

    c.take_bps = 30_001;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "take_bps must be at most 30_000",
    );
}

#[test]
fn take_must_exceed_payout_plus_protocol() {
    let mut c = config();

    // Exact equality with `payout_bps + protocol_bps` is refused; one more
    // basis point is enough.
    c.take_bps = 11_000;
    assert_panics_with(
        || terms::check_economics_absolute(&c, 500, 500),
        "take_bps must exceed payout_bps + protocol_bps",
    );

    c.take_bps = 11_001;
    terms::check_economics_absolute(&c, 500, 500);

    // The protocol term is what makes the inequality strict, so a zero
    // protocol cut still refuses equality with the payout.
    c.take_bps = 10_500;
    assert_panics_with(
        || terms::check_economics_absolute(&c, 0, 500),
        "take_bps must exceed payout_bps + protocol_bps",
    );

    c.take_bps = 10_501;
    terms::check_economics_absolute(&c, 0, 500);
}

#[test]
fn reversion_bps_range() {
    let mut c = config();

    c.reversion_bps = 1;
    terms::check_config_absolute(&c);

    c.reversion_bps = 10_000;
    terms::check_config_absolute(&c);

    c.reversion_bps = 0;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "reversion_bps must be in (0, 10_000]",
    );

    c.reversion_bps = 10_001;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "reversion_bps must be in (0, 10_000]",
    );
}

#[test]
fn max_reversion_weeks_ceiling() {
    let mut c = config();

    c.max_reversion_weeks = 52;
    terms::check_config_absolute(&c);

    c.max_reversion_weeks = 53;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "max_reversion_weeks must be at most 52",
    );
}

#[test]
fn cooldown_secs_ceiling() {
    let mut c = config();

    c.cooldown_secs = 604_800;
    terms::check_config_absolute(&c);

    c.cooldown_secs = 604_801;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "cooldown_secs must be at most 604_800",
    );
}

#[test]
fn protocol_bps_ceiling() {
    let mut c = config();
    // `payout_bps + protocol_bps` must stay strictly below `take_bps` for the
    // ceiling assertion itself to be the one that fires.
    c.payout_bps = 19_000;
    c.take_bps = 30_000;

    terms::check_economics_absolute(&c, 10_000, 500);

    assert_panics_with(
        || terms::check_economics_absolute(&c, 10_001, 500),
        "protocol_bps must be at most 10_000",
    );
}

#[test]
fn protocol_rent_bps_ceiling() {
    let mut c = config();
    // Leave room for the full protocol rent leg so the rent band still holds
    // at the ceiling (0 + 4_000) and the ceiling check fires first above it.
    c.site_rent_bps = 0;

    terms::check_economics_absolute(&c, 500, 4_000);

    assert_panics_with(
        || terms::check_economics_absolute(&c, 500, 4_001),
        "protocol_rent_bps must be at most 4_000",
    );
}

#[test]
fn total_rent_band_endpoints() {
    let mut c = config();

    c.site_rent_bps = 1_000;
    terms::check_economics_absolute(&c, 500, 0);

    c.site_rent_bps = 3_500;
    terms::check_economics_absolute(&c, 500, 500);

    c.site_rent_bps = 999;
    assert_panics_with(
        || terms::check_economics_absolute(&c, 500, 0),
        "site_rent_bps + protocol_rent_bps must be in [1_000, 4_000]",
    );

    c.site_rent_bps = 3_501;
    assert_panics_with(
        || terms::check_economics_absolute(&c, 500, 500),
        "site_rent_bps + protocol_rent_bps must be in [1_000, 4_000]",
    );
}

#[test]
fn max_rental_term_band() {
    let mut c = config();

    c.max_rental_term_secs = 3_600;
    terms::check_rent_terms(&c, 500);

    c.max_rental_term_secs = 31_536_000;
    terms::check_rent_terms(&c, 500);

    c.max_rental_term_secs = 3_599;
    assert_panics_with(
        || terms::check_rent_terms(&c, 500),
        "max_rental_term_secs must be in [3_600, 31_536_000]",
    );

    c.max_rental_term_secs = 31_536_001;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "max_rental_term_secs must be in [3_600, 31_536_000]",
    );
}

#[test]
fn min_rent_bps_range() {
    let mut c = config();

    c.min_rent_bps = 1;
    terms::check_config_absolute(&c);

    c.min_rent_bps = 10_000;
    terms::check_config_absolute(&c);

    c.min_rent_bps = 0;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "min_rent_bps must be in (0, 10_000]",
    );

    c.min_rent_bps = 10_001;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "min_rent_bps must be in (0, 10_000]",
    );
}

#[test]
fn total_rent_bps_saturates_on_overflow() {
    assert_eq!(terms::total_rent_bps(1_000, 0), 1_000);
    assert_eq!(terms::total_rent_bps(3_000, 1_000), 4_000);
    assert_eq!(terms::total_rent_bps(u64::MAX, 1), u64::MAX);

    let mut c = config();
    c.site_rent_bps = u64::MAX;
    assert_panics_with(
        || terms::check_rent_terms(&c, 1),
        "site_rent_bps + protocol_rent_bps must be in [1_000, 4_000]",
    );
}

// ----------------------------------------------------------------- ratchets

#[test]
fn ratchet_take_bps_only_decreases() {
    let prev = config();
    let mut next = config();

    terms::check_economics(&prev, &next, true);

    next.take_bps = prev.take_bps - 1;
    terms::check_economics(&prev, &next, true);

    next.take_bps = prev.take_bps + 1;
    assert_panics_with(
        || terms::check_economics(&prev, &next, true),
        "take_bps may only decrease while terms are locked",
    );
}

#[test]
fn ratchet_payout_bps_only_increases() {
    let prev = config();
    let mut next = config();

    terms::check_economics(&prev, &next, true);

    next.payout_bps = prev.payout_bps + 1;
    terms::check_economics(&prev, &next, true);

    next.payout_bps = prev.payout_bps - 1;
    assert_panics_with(
        || terms::check_economics(&prev, &next, true),
        "payout_bps may only increase while terms are locked",
    );
}

#[test]
fn ratchet_reversion_bps_only_slows() {
    let prev = config();
    let mut next = config();

    terms::check_economics(&prev, &next, true);

    // A larger bps decays more slowly per week.
    next.reversion_bps = prev.reversion_bps + 1;
    terms::check_economics(&prev, &next, true);

    next.reversion_bps = prev.reversion_bps - 1;
    assert_panics_with(
        || terms::check_economics(&prev, &next, true),
        "reversion_bps may only increase",
    );
}

#[test]
fn ratchet_max_reversion_weeks_only_shrinks() {
    let prev = config();
    let mut next = config();

    terms::check_economics(&prev, &next, true);

    next.max_reversion_weeks = prev.max_reversion_weeks - 1;
    terms::check_economics(&prev, &next, true);

    next.max_reversion_weeks = prev.max_reversion_weeks + 1;
    assert_panics_with(
        || terms::check_economics(&prev, &next, true),
        "max_reversion_weeks may only decrease while terms are locked",
    );
}

#[test]
fn ratchet_cooldown_secs_only_shrinks() {
    let prev = config();
    let mut next = config();

    terms::check_economics(&prev, &next, true);

    next.cooldown_secs = prev.cooldown_secs - 1;
    terms::check_economics(&prev, &next, true);

    next.cooldown_secs = prev.cooldown_secs + 1;
    assert_panics_with(
        || terms::check_economics(&prev, &next, true),
        "cooldown_secs may only decrease while terms are locked",
    );
}

#[test]
fn ratchets_are_skipped_before_the_lock() {
    let prev = config();
    let mut next = config();

    // Every ratchet direction inverted at once; the absolute clamps still hold.
    next.take_bps = prev.take_bps + 1_000;
    next.payout_bps = prev.payout_bps - 500;
    next.reversion_bps = prev.reversion_bps - 1_000;
    next.max_reversion_weeks = prev.max_reversion_weeks + 1;
    next.cooldown_secs = prev.cooldown_secs + 1;
    terms::check_economics(&prev, &next, false);

    // Absolute clamps are not skipped before the lock.
    next.payout_bps = 9_999;
    assert_panics_with(
        || terms::check_economics(&prev, &next, false),
        "payout_bps must be at least 10_000",
    );
}

#[test]
fn ratchet_take_panic_precedes_absolute_payout_panic() {
    let prev = config();
    let mut next = config();

    next.take_bps = prev.take_bps + 1; // ratchet violation
    next.payout_bps = 9_000; // absolute violation

    assert_panics_with(
        || terms::check_economics(&prev, &next, true),
        "take_bps may only decrease while terms are locked",
    );
}

#[test]
fn ratchet_payout_panic_precedes_absolute_reversion_panic() {
    let prev = config();
    let mut next = config();

    next.payout_bps = prev.payout_bps - 1; // ratchet violation
    next.reversion_bps = 0; // absolute violation

    assert_panics_with(
        || terms::check_economics(&prev, &next, true),
        "payout_bps may only increase while terms are locked",
    );
}

// -------------------------------------------------------------- floor policy

#[test]
fn floor_delta_absolute_band() {
    let mut c = config();

    for delta in [1_u64, 1_000, 2_000] {
        c.floor_delta_bps = delta;
        terms::check_config_absolute(&c);
    }

    // The origin rejects 0 outright: `floorDeltaBps == 0 || > 2_000` reverts.
    c.floor_delta_bps = 0;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "floor_delta_bps must be in [1, 2_000]",
    );

    c.floor_delta_bps = 2_001;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "floor_delta_bps must be in [1, 2_000]",
    );
    assert_panics_with(
        || terms::check_economics_absolute(&c, 500, 500),
        "floor_delta_bps must be in [1, 2_000]",
    );
}

#[test]
fn floor_change_cooldown_floor() {
    let mut c = config();

    c.floor_change_cooldown_secs = 86_400;
    terms::check_config_absolute(&c);

    c.floor_change_cooldown_secs = 86_399;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "floor_change_cooldown_secs must be at least 86_400",
    );

    // With `prev == next` the ratchet block is neutral, so the absolute clamp
    // is the one that fires.
    assert_panics_with(
        || terms::check_floor_policy(&c, &c),
        "floor_change_cooldown_secs must be at least 86_400",
    );
}

#[test]
fn max_ask_bps_floor() {
    let mut c = config();

    c.max_ask_bps = 10_000;
    terms::check_config_absolute(&c);

    c.max_ask_bps = 9_999;
    assert_panics_with(
        || terms::check_config_absolute(&c),
        "max_ask_bps must be at least 10_000",
    );
}

#[test]
fn floor_delta_change_requires_band_and_cooldown() {
    let prev = config();
    let mut next = config();

    // Unchanged delta always passes.
    terms::check_floor_policy(&prev, &next);

    // In-band change with the cooldown unchanged.
    next.floor_delta_bps = 1_000;
    terms::check_floor_policy(&prev, &next);

    // In-band change with a longer cooldown.
    next.floor_change_cooldown_secs = 86_401;
    terms::check_floor_policy(&prev, &next);

    // A change outside the legal change band is refused.
    next.floor_delta_bps = 2_001;
    assert_panics_with(
        || terms::check_floor_policy(&prev, &next),
        "floor_delta_bps may only change inside [1, 2_000]",
    );

    // Disabling the lever is not a tightening: `0` is outside the change band.
    next.floor_delta_bps = 0;
    assert_panics_with(
        || terms::check_floor_policy(&prev, &next),
        "floor_delta_bps may only change inside [1, 2_000]",
    );

    // An in-band change may not shorten the floor cooldown.
    next.floor_delta_bps = 1_000;
    next.floor_change_cooldown_secs = 86_399;
    assert_panics_with(
        || terms::check_floor_policy(&prev, &next),
        "floor_change_cooldown_secs shrinks",
    );
}

#[test]
fn floor_policy_ratchets_tighten_only() {
    let prev = config();
    let mut next = config();

    // Longer cooldown, unchanged delta.
    next.floor_change_cooldown_secs = prev.floor_change_cooldown_secs + 1;
    terms::check_floor_policy(&prev, &next);

    // Shorter cooldown, unchanged delta.
    next = config();
    next.floor_change_cooldown_secs = prev.floor_change_cooldown_secs - 1;
    assert_panics_with(
        || terms::check_floor_policy(&prev, &next),
        "floor_change_cooldown_secs may only increase while terms are locked",
    );

    // `max_ask_bps` may tighten but not rise.
    next = config();
    next.max_ask_bps = prev.max_ask_bps - 1;
    terms::check_floor_policy(&prev, &next);

    next.max_ask_bps = prev.max_ask_bps + 1;
    assert_panics_with(
        || terms::check_floor_policy(&prev, &next),
        "max_ask_bps may only decrease while terms are locked",
    );
}

// ---------------------------------------------------------------- rent terms

#[test]
fn check_rent_terms_rechecks_band_and_term() {
    let mut c = config();

    // Total exactly at the lower end.
    c.site_rent_bps = 500;
    terms::check_rent_terms(&c, 500);

    // The mutable term is re-checked on the same call.
    c.max_rental_term_secs = 3_599;
    assert_panics_with(
        || terms::check_rent_terms(&c, 500),
        "max_rental_term_secs must be in [3_600, 31_536_000]",
    );

    // And so is the band, on the total.
    c.max_rental_term_secs = 31_536_000;
    c.site_rent_bps = 3_501;
    assert_panics_with(
        || terms::check_rent_terms(&c, 500),
        "site_rent_bps + protocol_rent_bps must be in [1_000, 4_000]",
    );
}
