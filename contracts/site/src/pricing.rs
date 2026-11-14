//! Pure `U256` money math, parity-tested against `packages/sdk/src/pricing.ts` (3,024-row `uint256` grid, 1:1 with `base_price` / `141`).
//! Checked math panics where Solidity reverts; weekly decay iterates for per-step truncation.

use primitive_types::U256;

pub const BPS_DENOMINATOR: u64 = 10_000;
pub const SECONDS_PER_WEEK: u64 = 604_800;

/// `a * b`, panicking where Solidity's checked multiply reverts.
#[inline]
fn mul(a: U256, b: U256) -> U256 {
    a.checked_mul(b)
        .expect("pricing: multiply overflows uint256")
}

/// `a - b`, panicking where Solidity's checked subtract reverts.
#[inline]
fn sub(a: U256, b: U256) -> U256 {
    a.checked_sub(b)
        .expect("pricing: subtract underflows uint256")
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
    let bps = U256::from(BPS_DENOMINATOR);
    let decay = U256::from(decay_bps);
    let effective_floor = if clamped_weeks == 0 {
        last_price.max(base_price)
    } else if decay_bps == BPS_DENOMINATOR {
        let _ = mul(last_price, bps);
        last_price.max(base_price)
    } else if last_price <= base_price && decay_bps <= BPS_DENOMINATOR {
        let _ = mul(last_price, decay);
        base_price
    } else if decay_bps == 0 {
        base_price
    } else {
        let mut decayed = last_price;
        for _ in 0..clamped_weeks {
            decayed = mul(decayed, decay) / bps;
            if decayed <= base_price {
                decayed = base_price;
                break;
            }
        }
        decayed.max(base_price)
    };
    let price = mul(effective_floor, U256::from(take_bps)) / bps;
    TakeQuote {
        price,
        effective_floor,
    }
}

/// Mirrors `Pricing.resolveReversionBase`: a posted ask replaces the reversion base.
pub fn resolve_reversion_base(ask_floor: U256, last_price: U256) -> U256 {
    if ask_floor != U256::zero() {
        ask_floor
    } else {
        last_price
    }
}

/// Mirrors `Pricing.askCeiling`: the highest ask an owner may post.
pub fn ask_ceiling(last_price: U256, base_price: U256, max_ask_bps: u64) -> U256 {
    let anchor = if last_price > base_price {
        last_price
    } else {
        base_price
    };
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
    let payout = if is_unclaimed {
        U256::zero()
    } else {
        bps_of(effective_floor, payout_bps)
    };
    let protocol_cut = bps_of(effective_floor, protocol_bps);
    let site_cut = sub(sub(charged, payout), protocol_cut);
    Split {
        charged,
        payout,
        protocol_cut,
        site_cut,
    }
}

/// Mirrors `origin site._computeElapsedWeeks`; panics on underflow.
pub fn compute_elapsed_weeks(now_ts: u64, last_purchase_ts: u64) -> u64 {
    now_ts
        .checked_sub(last_purchase_ts)
        .expect("pricing: nowTs must be >= lastPurchaseTs")
        / SECONDS_PER_WEEK
}

/// Booking boundary: rejects `U256` values exceeding `u128`.
pub fn to_u128(value: U256, what: &str) -> u128 {
    u128::try_from(value).unwrap_or_else(|_| panic!("{what} exceeds u128"))
}
