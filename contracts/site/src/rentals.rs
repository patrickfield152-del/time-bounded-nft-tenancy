//! Rent arithmetic, ported 1:1 from the origin `origin rentals library` through its byte-identical
//! TypeScript twin (`packages/sdk/src/rentals.ts`).
//!
//! Rates are per DAY, durations are in SECONDS, and nothing here moves money: the site contract
//! books the legs. Pure functions stay in seconds exactly like the vector grid; the contract
//! converts nanoseconds only at its boundary.
//!
//! The three state-machine predicates at the bottom are the guards the site applies around
//! [`rent_cost`]/[`rent_split`]: they return a verdict or an amount and never mutate anything.

use primitive_types::U256;

use crate::pricing::BPS_DENOMINATOR;

/// Seconds in a day; rent rates are quoted per day.
pub const SECONDS_PER_DAY: u64 = 86_400;
/// The shortest term the contract will open or extend (1 hour).
pub const MIN_RENTAL_DURATION: u64 = 3_600;

#[inline]
fn mul(a: U256, b: U256) -> U256 {
    a.checked_mul(b).expect("rentals: multiply overflows uint256")
}

#[inline]
fn sub(a: U256, b: U256) -> U256 {
    a.checked_sub(b).expect("rentals: subtract underflows uint256")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accrual {
    /// Rent the owner has earned so far, claimed or not.
    pub accrued: U256,
    /// The unearned remainder — what a buyer inherits.
    pub unaccrued: U256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RentSplit {
    pub protocol_cut: U256,
    pub site_cut: U256,
    /// What the owner keeps after both cuts; derived by subtraction so the legs sum to `cost`.
    pub net: U256,
}

/// Mirrors `origin rentals library.accruedOf`: linear accrual, no segment history.
pub fn accrued_of(prepaid: U256, start: u64, expiry: u64, now_ts: u64) -> U256 {
    if expiry == 0 {
        return U256::zero();
    }
    let end = now_ts.min(expiry);
    if end <= start {
        return U256::zero();
    }
    mul(prepaid, U256::from(end - start)) / U256::from(expiry - start)
}

/// Both legs at once; `accrued + unaccrued == prepaid` holds exactly.
pub fn accrual(prepaid: U256, start: u64, expiry: u64, now_ts: u64) -> Accrual {
    let accrued = accrued_of(prepaid, start, expiry, now_ts);
    Accrual { accrued, unaccrued: sub(prepaid, accrued) }
}

/// Mirrors `origin rentals library.rentCost`: gross rent for a term.
pub fn rent_cost(rate_per_day: U256, duration_secs: u64) -> U256 {
    mul(rate_per_day, U256::from(duration_secs)) / U256::from(SECONDS_PER_DAY)
}

/// Mirrors `origin rentals library.rentSplit`; both cuts key to the GROSS cost and `net` is what remains.
pub fn rent_split(cost: U256, protocol_rent_bps: u64, fee_bps: u64) -> RentSplit {
    let protocol_cut = mul(cost, U256::from(protocol_rent_bps)) / U256::from(BPS_DENOMINATOR);
    let site_cut = mul(cost, U256::from(fee_bps)) / U256::from(BPS_DENOMINATOR);
    RentSplit { protocol_cut, site_cut, net: sub(sub(cost, protocol_cut), site_cut) }
}

/// True when `duration_secs` sits inside `[MIN_RENTAL_DURATION, max_duration_secs]`.
///
/// Mirrors the `TermTooLong` half of `origin rentals library.list`/`open`/`extend`: the floor keeps a per-day
/// rate from being charged against an unrepresentably short term, and the ceiling is the caller's
/// cap — `max_rental_term_secs` for a listing, the listing's own `max_duration_secs` for a rent or
/// an extension. A cap of zero (no listing) admits nothing.
pub fn duration_ok(duration_secs: u64, max_duration_secs: u64) -> bool {
    duration_secs >= MIN_RENTAL_DURATION && duration_secs <= max_duration_secs
}

/// Mirrors the `RateBelowFloor` half of `origin rentals library.list`: a daily rate must clear
/// `effective_floor * min_rent_bps / 10_000`.
///
/// Keyed to the EFFECTIVE floor, not `base_price`, so the rent floor tracks the same decayed number
/// the take price uses. `min_rent_bps` is the site's rent floor in bps of that floor, validated to
/// `(0, 10_000]`.
pub fn hourly_floor(rate_per_day: U256, min_rent_bps: u64, effective_floor: U256) -> bool {
    rate_per_day >= mul(effective_floor, U256::from(min_rent_bps)) / U256::from(BPS_DENOMINATOR)
}

/// The carry a settled restart takes forward: `prepaid` minus everything the term has earned, or
/// already claimed, whichever is larger.
///
/// The site settles first, so `claimed == accrued` on entry and the result is exactly
/// [`accrual`]'s `unaccrued` leg: rent the tenant paid for but the term never earned, which must
/// roll into the new term rather than be refunded. `claimed > prepaid` (or an accrual above
/// `prepaid`) is an invariant violation and panics like every other subtraction here.
pub fn extend_carry(prepaid: U256, claimed: U256, start: u64, expiry: u64, now_ts: u64) -> U256 {
    let accrued = accrued_of(prepaid, start, expiry, now_ts);
    let settled = if claimed > accrued { claimed } else { accrued };
    sub(prepaid, settled)
}
