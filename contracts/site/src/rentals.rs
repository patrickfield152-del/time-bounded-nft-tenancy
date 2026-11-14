//! Rent math (`origin rentals library` / `packages/sdk/src/rentals.ts` twin): per-day `prepaid` rates, `duration_secs` durations, and predicates (`rent_cost`, `rent_split`).

use primitive_types::U256;

use crate::pricing::BPS_DENOMINATOR;

/// Seconds in a day; rent rates are quoted per day.
pub const SECONDS_PER_DAY: u64 = 86_400;
/// The shortest term the contract will open or extend (1 hour).
pub const MIN_RENTAL_DURATION: u64 = 3_600;

#[inline]
fn mul(a: U256, b: U256) -> U256 {
    a.checked_mul(b)
        .expect("rentals: multiply overflows uint256")
}

#[inline]
fn sub(a: U256, b: U256) -> U256 {
    a.checked_sub(b)
        .expect("rentals: subtract underflows uint256")
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
    Accrual {
        accrued,
        unaccrued: sub(prepaid, accrued),
    }
}

/// Mirrors `origin rentals library.rentCost`: gross rent for a term.
pub fn rent_cost(rate_per_day: U256, duration_secs: u64) -> U256 {
    mul(rate_per_day, U256::from(duration_secs)) / U256::from(SECONDS_PER_DAY)
}

/// Mirrors `origin rentals library.rentSplit`; both cuts key to the GROSS cost and `net` is what remains.
pub fn rent_split(cost: U256, protocol_rent_bps: u64, fee_bps: u64) -> RentSplit {
    let protocol_cut = mul(cost, U256::from(protocol_rent_bps)) / U256::from(BPS_DENOMINATOR);
    let site_cut = mul(cost, U256::from(fee_bps)) / U256::from(BPS_DENOMINATOR);
    RentSplit {
        protocol_cut,
        site_cut,
        net: sub(sub(cost, protocol_cut), site_cut),
    }
}

/// Origin `TermTooLong` (`origin rentals library.list`/`open`/`extend`): true inside `[MIN_RENTAL_DURATION, max_duration_secs]` (`max_rental_term_secs` or `max_duration_secs`).
pub fn duration_ok(duration_secs: u64, max_duration_secs: u64) -> bool {
    duration_secs >= MIN_RENTAL_DURATION && duration_secs <= max_duration_secs
}

/// Origin `RateBelowFloor` (`origin rentals library.list`): clears `effective_floor * min_rent_bps / 10_000` (not `base_price`; `min_rent_bps` in `(0, 10_000]`).
pub fn hourly_floor(rate_per_day: U256, min_rent_bps: u64, effective_floor: U256) -> bool {
    rate_per_day >= mul(effective_floor, U256::from(min_rent_bps)) / U256::from(BPS_DENOMINATOR)
}

/// Unaccrued carry on restart (`claimed == accrued`, `unaccrued` rolled forward; panics if `claimed > prepaid`), matching [`accrual`].
pub fn extend_carry(prepaid: U256, claimed: U256, start: u64, expiry: u64, now_ts: u64) -> U256 {
    let accrued = accrued_of(prepaid, start, expiry, now_ts);
    let settled = if claimed > accrued { claimed } else { accrued };
    sub(prepaid, settled)
}
