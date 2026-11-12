//! Pure money math, ported 1:1 from the origin `Pricing.sol` through its byte-identical
//! TypeScript twin (`packages/sdk/src/pricing.ts`).
//!
//! Everything here is `U256` and total: the 3,024-row parity grid includes `uint256` overflow
//! rows, so checked arithmetic must panic exactly where Solidity reverts. The weekly decay loop is
//! iterative on purpose: truncating once per iteration is load-bearing, and a closed form is a
//! different number.

use primitive_types::U256;

pub const BPS_DENOMINATOR: u64 = 10_000;
pub const SECONDS_PER_WEEK: u64 = 604_800;

/// `a * b`, panicking where Solidity's checked multiply reverts.
#[inline]
fn mul(a: U256, b: U256) -> U256 {
    a.checked_mul(b).expect("pricing: multiply overflows uint256")
}

/// `a - b`, panicking where Solidity's checked subtract reverts.
#[inline]
fn sub(a: U256, b: U256) -> U256 {
    a.checked_sub(b).expect("pricing: subtract underflows uint256")
}

/// `amount * bps / 10_000` — one checked multiply, one truncating divide, in that order.
#[inline]
fn bps_of(amount: U256, bps: u64) -> U256 {
    mul(amount, U256::from(bps)) / U256::from(BPS_DENOMINATOR)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeQuote {
    /// What a taker pays to displace the current owner.
    pub price: U256,
    /// `max(decayed, base_price)` — what a claim pays and what every split leg is keyed to.
    pub effective_floor: U256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    /// Effective floor on a claim, take price on a take. This is what `last_price` records.
    pub charged: U256,
    /// To the displaced owner; zero on a claim.
    pub payout: U256,
    /// To the protocol treasury on every buy, claims included.
    pub protocol_cut: U256,
    /// The remainder, to the site.
    pub site_cut: U256,
}

/// Mirrors `Pricing.computeTakePrice`.
pub fn compute_take_price(
    last_price: U256,
    base_price: U256,
    elapsed_weeks: u64,
    decay_bps: u64,
    take_bps: u64,
    max_decay_weeks: u64,
) -> TakeQuote {
    let clamped_weeks = elapsed_weeks.min(max_decay_weeks);
    let mut decayed = last_price;
    for _ in 0..clamped_weeks {
        decayed = mul(decayed, U256::from(decay_bps)) / U256::from(BPS_DENOMINATOR);
    }
    let effective_floor = if decayed > base_price { decayed } else { base_price };
    let price = mul(effective_floor, U256::from(take_bps)) / U256::from(BPS_DENOMINATOR);
    TakeQuote { price, effective_floor }
}

/// Mirrors `Pricing.resolveReversionBase`: a posted ask replaces the reversion base.
pub fn resolve_reversion_base(ask_floor: U256, last_price: U256) -> U256 {
    if ask_floor != U256::zero() { ask_floor } else { last_price }
}

/// Mirrors `Pricing.askCeiling`: the highest ask an owner may post.
pub fn ask_ceiling(last_price: U256, base_price: U256, max_ask_bps: u64) -> U256 {
    let anchor = if last_price > base_price { last_price } else { base_price };
    bps_of(anchor, max_ask_bps)
}

/// Mirrors `Pricing.computeSplit`; every leg is keyed to `effective_floor`, never to `charged`.
pub fn compute_split(
    effective_floor: U256,
    price: U256,
    is_unclaimed: bool,
    payout_bps: u64,
    protocol_bps: u64,
) -> Split {
    let charged = if is_unclaimed { effective_floor } else { price };
    let payout = if is_unclaimed { U256::zero() } else { bps_of(effective_floor, payout_bps) };
    let protocol_cut = bps_of(effective_floor, protocol_bps);
    let site_cut = sub(sub(charged, payout), protocol_cut);
    Split { charged, payout, protocol_cut, site_cut }
}

/// Mirrors `origin site._computeElapsedWeeks`; panics on underflow instead of quoting an undecayed
/// price.
pub fn compute_elapsed_weeks(now_ts: u64, last_purchase_ts: u64) -> u64 {
    now_ts.checked_sub(last_purchase_ts).expect("pricing: nowTs must be >= lastPurchaseTs")
        / SECONDS_PER_WEEK
}

/// Booking boundary: a `U256` that does not fit `u128` can never be paid by a NEP-141 transfer.
pub fn to_u128(value: U256, what: &str) -> u128 {
    u128::try_from(value).unwrap_or_else(|_| panic!("{what} exceeds u128"))
}
