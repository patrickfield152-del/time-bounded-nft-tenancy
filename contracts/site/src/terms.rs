//! [`SiteConfig`] validation policy: clamps and `origin terms library.checkEconomics` / `checkFloorPolicy` ratchets (`prev == next` degrades to absolute).
//! Once terms lock, changes tighten protections (`take_bps > payout_bps + protocol_bps`, `2_000`, `1`, `2`, `3`, `4`); `check_economics` runs ratchets before `checkEconomicsAbsolute`.

use crate::SiteConfig;

/// 100% in basis points.
const BPS_DENOMINATOR: u64 = 10_000;
/// Origin `MAX_TAKE_BPS` in the origin terms library.
const MAX_TAKE_BPS: u64 = 30_000;
/// Origin `origin terms library.MAX_REVERSION_WEEKS_CEILING`.
const MAX_REVERSION_WEEKS: u32 = 52;
/// Origin `origin terms library.MAX_COOLDOWN_SECS` (`7 days`).
const MAX_COOLDOWN_SECS: u64 = 7 * 24 * 60 * 60;
/// Origin `origin terms library.MIN_MAX_ASK_BPS`.
const MIN_MAX_ASK_BPS: u64 = 10_000;
/// Origin `origin terms library.MAX_FLOOR_DELTA_BPS_CEILING`.
const MAX_FLOOR_DELTA_BPS: u64 = 2_000;
/// Origin `origin terms library.MIN_FLOOR_CHANGE_COOLDOWN` (`24 hours`).
const MIN_FLOOR_CD_SECS: u64 = 24 * 60 * 60;

/// Protocol take cap (`protocol_bps <= 10_000`).
const MAX_PROTOCOL_BPS: u64 = BPS_DENOMINATOR;
/// Protocol rent cap (`protocol_rent_bps <= 4_000`).
const MAX_PROTOCOL_RENT_BPS: u64 = 4_000;
/// Lower end of the rent band on `site_rent_bps + protocol_rent_bps`.
const MIN_TOTAL_RENT_BPS: u64 = 1_000;
/// Upper end of the rent band on `site_rent_bps + protocol_rent_bps`.
const MAX_TOTAL_RENT_BPS: u64 = 4_000;
/// `max_rental_term_secs` floor (`1 hour`).
const MIN_RENTAL_TERM_SECS: u64 = 3_600;
/// `max_rental_term_secs` ceiling (`365 days`).
const MAX_RENTAL_TERM_SECS: u64 = 365 * 24 * 60 * 60;

/// Evaluates config clamps without protocol arguments (`check_economics_absolute` adds protocol clamps).
pub fn check_config_absolute(c: &SiteConfig) {
    chk_econ_fields(c, None);
    chk_floor_abs(c);
    chk_rent_term(c);
    chk_min_rent(c);
}

/// Runs ratchets (if `locked`) before absolute clamps.
pub fn check_economics(prev: &SiteConfig, next: &SiteConfig, locked: bool) {
    if locked {
        chk_ratchets(prev, next);
    }
    check_config_absolute(next);
}

/// Full absolute validation: economics, protocol caps, rent band, and floor policy.
pub fn check_economics_absolute(c: &SiteConfig, protocol_bps: u64, protocol_rent_bps: u64) {
    chk_econ_fields(c, Some(protocol_bps));
    assert!(
        protocol_bps <= MAX_PROTOCOL_BPS,
        "protocol_bps must be at most 10_000"
    );
    assert!(
        protocol_rent_bps <= MAX_PROTOCOL_RENT_BPS,
        "protocol_rent_bps must be at most 4_000"
    );
    chk_rent_band(c, protocol_rent_bps);
    chk_rent_term(c);
    chk_min_rent(c);
    chk_floor_abs(c);
}

/// Origin `origin terms library.checkFloorPolicy`: validates `floor_delta_bps` (`prev` -> `[1, 2_000]`, never `0`), `floor_change_cooldown_secs`, and `max_ask_bps`.
pub fn check_floor_policy(prev: &SiteConfig, next: &SiteConfig) {
    if next.floor_delta_bps != prev.floor_delta_bps {
        assert!(
            (1..=MAX_FLOOR_DELTA_BPS).contains(&next.floor_delta_bps),
            "floor_delta_bps may only change inside [1, 2_000] while terms are locked"
        );
        assert!(
            next.floor_change_cooldown_secs >= prev.floor_change_cooldown_secs,
            "floor_delta_bps may not change while floor_change_cooldown_secs shrinks"
        );
    }
    assert!(
        next.floor_change_cooldown_secs >= prev.floor_change_cooldown_secs,
        "floor_change_cooldown_secs may only increase while terms are locked"
    );
    assert!(
        next.max_ask_bps <= prev.max_ask_bps,
        "max_ask_bps may only decrease while terms are locked"
    );
    chk_floor_abs(next);
}

/// Validates `set_rental_terms` (`site_rent_bps + protocol_rent_bps` and `max_rental_term_secs`).
pub fn check_rent_terms(c: &SiteConfig, protocol_rent_bps: u64) {
    chk_rent_band(c, protocol_rent_bps);
    chk_rent_term(c);
}

/// `site_rent_bps + protocol_rent_bps`, saturating `u64` so overflow fails `[1_000, 4_000]`.
pub fn total_rent_bps(site_rent_bps: u64, protocol_rent_bps: u64) -> u64 {
    site_rent_bps.saturating_add(protocol_rent_bps)
}

/// Shared economics clamps; `protocol_bps` is `Some` when take-vs-payout applies.
fn chk_econ_fields(c: &SiteConfig, protocol_bps: Option<u64>) {
    // A displaced owner always recovers at least the current effective floor,
    // so a site can only monetize the spread, never the principal.
    assert!(
        c.payout_bps >= BPS_DENOMINATOR,
        "payout_bps must be at least 10_000"
    );
    assert!(
        c.take_bps <= MAX_TAKE_BPS,
        "take_bps must be at most 30_000"
    );
    if let Some(protocol_bps) = protocol_bps {
        // Strictly stronger than `take_bps > payout_bps`: the site's cut is
        // `charged - payout - protocol`, so `<=` would underflow every take.
        // A saturating sum can never be exceeded, so an overflow is rejected
        // by this clamp rather than wrapping.
        assert!(
            c.take_bps > c.payout_bps.saturating_add(protocol_bps),
            "take_bps must exceed payout_bps + protocol_bps"
        );
    }
    assert!(
        c.reversion_bps > 0 && c.reversion_bps <= BPS_DENOMINATOR,
        "reversion_bps must be in (0, 10_000]"
    );
    assert!(
        c.max_reversion_weeks <= MAX_REVERSION_WEEKS,
        "max_reversion_weeks must be at most 52"
    );
    assert!(
        c.cooldown_secs <= MAX_COOLDOWN_SECS,
        "cooldown_secs must be at most 604_800 (7 days)"
    );
}

/// Holder-safe ratchets evaluated after terms lock.
fn chk_ratchets(prev: &SiteConfig, next: &SiteConfig) {
    assert!(
        next.take_bps <= prev.take_bps,
        "take_bps may only decrease while terms are locked"
    );
    assert!(
        next.payout_bps >= prev.payout_bps,
        "payout_bps may only increase while terms are locked"
    );
    // A larger `reversion_bps` decays more slowly per week, which protects the
    // payout a holder is owed; the origin rejects `next < current`, so this
    // ratchet is up-only. (Decay speed goes down, the stored bps goes up.)
    assert!(
        next.reversion_bps >= prev.reversion_bps,
        "reversion_bps may only increase (slower decay) while terms are locked"
    );
    assert!(
        next.max_reversion_weeks <= prev.max_reversion_weeks,
        "max_reversion_weeks may only decrease while terms are locked"
    );
    assert!(
        next.cooldown_secs <= prev.cooldown_secs,
        "cooldown_secs may only decrease while terms are locked"
    );
}

/// Absolute floor-policy clamps.
fn chk_floor_abs(c: &SiteConfig) {
    assert!(
        (1..=MAX_FLOOR_DELTA_BPS).contains(&c.floor_delta_bps),
        "floor_delta_bps must be in [1, 2_000]"
    );
    assert!(
        c.floor_change_cooldown_secs >= MIN_FLOOR_CD_SECS,
        "floor_change_cooldown_secs must be at least 86_400 (24 hours)"
    );
    // `10_000` is down-only; below it an owner could not post an ask at their
    // own cost.
    assert!(
        c.max_ask_bps >= MIN_MAX_ASK_BPS,
        "max_ask_bps must be at least 10_000"
    );
}

/// Rent band on total rent bps.
fn chk_rent_band(c: &SiteConfig, protocol_rent_bps: u64) {
    let total = total_rent_bps(c.site_rent_bps, protocol_rent_bps);
    assert!(
        (MIN_TOTAL_RENT_BPS..=MAX_TOTAL_RENT_BPS).contains(&total),
        "site_rent_bps + protocol_rent_bps must be in [1_000, 4_000]"
    );
}

fn chk_rent_term(c: &SiteConfig) {
    assert!(
        (MIN_RENTAL_TERM_SECS..=MAX_RENTAL_TERM_SECS).contains(&c.max_rental_term_secs),
        "max_rental_term_secs must be in [3_600, 31_536_000]"
    );
}

fn chk_min_rent(c: &SiteConfig) {
    assert!(
        c.min_rent_bps > 0 && c.min_rent_bps <= BPS_DENOMINATOR,
        "min_rent_bps must be in (0, 10_000]"
    );
}
