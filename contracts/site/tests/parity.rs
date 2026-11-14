//! Vector parity: the Rust port must reproduce the committed Foundry grids byte-for-byte,
//! including the rows where Solidity reverts.
//!
//! Files: `vectors/price-vectors.jsonl` (3,024 rows) and `vectors/v2-vectors.jsonl` (690 rows),
//! generated once by `scripts/regen-vectors-from-origin.sh` and committed. A missing or empty file
//! is a hard failure, never a skip: a vacuous pass is worse than a red test.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use primitive_types::U256;
use serde::Deserialize;
use stele_site::pricing::{self, compute_split, compute_take_price};
use stele_site::rentals::{self, accrual, rent_cost, rent_split};

// Fixtures

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vectors")
}

fn read_jsonl<T: for<'de> Deserialize<'de>>(name: &str) -> Vec<T> {
    let path = vectors_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing vector file {} ({e}); run scripts/regen-vectors-from-origin.sh and commit it",
            path.display()
        )
    });
    let rows: Vec<T> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| panic!("bad row in {name}: {e}\n{line}"))
        })
        .collect();
    assert!(!rows.is_empty(), "{name} is empty");
    rows
}

fn dec(value: &str) -> U256 {
    U256::from_dec_str(value).unwrap_or_else(|_| panic!("not a decimal uint256: {value}"))
}

fn num(value: &str) -> u64 {
    value
        .parse()
        .unwrap_or_else(|_| panic!("not a u64: {value}"))
}

// Price grid

#[derive(Debug, Deserialize)]
struct Leg {
    charged: String,
    payout: String,
    protocol: String,
    site: String,
}

#[derive(Debug, Deserialize)]
struct PriceRow {
    take_bps: u64,
    payout_bps: u64,
    decay_bps: u64,
    max_decay_weeks: u64,
    protocol_bps: u64,
    base_price: String,
    last_price: String,
    elapsed_weeks: u64,
    overflow: bool,
    price: Option<String>,
    effective_floor: Option<String>,
    claim: Option<Leg>,
    take: Option<Leg>,
}

fn quote(row: &PriceRow) -> pricing::TakeQuote {
    compute_take_price(
        dec(&row.last_price),
        dec(&row.base_price),
        row.elapsed_weeks,
        row.decay_bps,
        row.take_bps,
        row.max_decay_weeks,
    )
}

fn describe(row: &PriceRow) -> String {
    format!(
        "take={} payout={} decay={} weeks<={} protocol={} base={} last={} elapsed={}",
        row.take_bps,
        row.payout_bps,
        row.decay_bps,
        row.max_decay_weeks,
        row.protocol_bps,
        row.base_price,
        row.last_price,
        row.elapsed_weeks
    )
}

#[test]
fn price_grid_is_wide_and_has_both_row_classes() {
    let rows: Vec<PriceRow> = read_jsonl("price-vectors.jsonl");
    assert!(rows.len() > 2_000, "grid too narrow: {} rows", rows.len());

    let configs = rows
        .iter()
        .map(|r| {
            format!(
                "{}/{}/{}/{}/{}",
                r.take_bps, r.payout_bps, r.decay_bps, r.max_decay_weeks, r.protocol_bps
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        configs.len() >= 12,
        "config space not swept: {} configs",
        configs.len()
    );

    assert!(
        rows.iter().any(|r| r.take_bps == 30_000),
        "take clamp end missing"
    );
    assert!(
        rows.iter().any(|r| r.payout_bps == 10_000),
        "payout clamp end missing"
    );
    assert!(
        rows.iter().any(|r| r.decay_bps == 10_000),
        "decay clamp end missing"
    );
    assert!(rows.iter().any(|r| r.decay_bps == 1), "decay floor missing");
    assert!(
        rows.iter().any(|r| r.max_decay_weeks == 0),
        "zero-week clamp missing"
    );
    assert!(
        rows.iter().any(|r| r.max_decay_weeks == 52),
        "week clamp end missing"
    );
    assert!(
        rows.iter().any(|r| r.protocol_bps == 0),
        "zero protocol row missing"
    );

    let overflow = rows.iter().filter(|r| r.overflow).count();
    assert!(overflow > 0, "no overflow rows");
    assert!(rows.len() - overflow > 1_000, "too few computable rows");
}

#[test]
fn price_and_effective_floor_match_every_computable_row() {
    let rows: Vec<PriceRow> = read_jsonl("price-vectors.jsonl");
    let mut checked = 0usize;

    for row in rows.iter().filter(|r| !r.overflow) {
        let quote = quote(row);
        let expected_price = row
            .price
            .as_deref()
            .unwrap_or_else(|| panic!("missing price — {}", describe(row)));
        let expected_floor = row
            .effective_floor
            .as_deref()
            .unwrap_or_else(|| panic!("missing effective_floor — {}", describe(row)));
        assert_eq!(
            quote.price.to_string(),
            expected_price,
            "price divergence — {}",
            describe(row)
        );
        assert_eq!(
            quote.effective_floor.to_string(),
            expected_floor,
            "floor divergence — {}",
            describe(row)
        );
        checked += 1;
    }

    assert!(checked > 1_000, "comparison was vacuous: {checked} rows");
}

#[test]
fn claim_and_take_splits_match_every_computable_row() {
    let rows: Vec<PriceRow> = read_jsonl("price-vectors.jsonl");

    for row in rows.iter().filter(|r| !r.overflow) {
        let quote = quote(row);
        for (branch, expected) in [("claim", row.claim.as_ref()), ("take", row.take.as_ref())] {
            let expected =
                expected.unwrap_or_else(|| panic!("missing {branch} split — {}", describe(row)));
            let is_unclaimed = branch == "claim";
            let split = compute_split(
                quote.effective_floor,
                quote.price,
                is_unclaimed,
                row.payout_bps,
                row.protocol_bps,
            );
            assert_eq!(
                split.charged.to_string(),
                expected.charged,
                "{branch} charged — {}",
                describe(row)
            );
            assert_eq!(
                split.payout.to_string(),
                expected.payout,
                "{branch} payout — {}",
                describe(row)
            );
            assert_eq!(
                split.protocol_cut.to_string(),
                expected.protocol,
                "{branch} protocol — {}",
                describe(row)
            );
            assert_eq!(
                split.site_cut.to_string(),
                expected.site,
                "{branch} site — {}",
                describe(row)
            );
        }
    }
}

#[test]
fn overflow_rows_revert_exactly_where_solidity_does() {
    let rows: Vec<PriceRow> = read_jsonl("price-vectors.jsonl");
    let overflowing: Vec<&PriceRow> = rows.iter().filter(|r| r.overflow).collect();
    assert!(!overflowing.is_empty(), "no overflow rows to check");

    for row in overflowing {
        let reverted = catch_unwind(AssertUnwindSafe(|| {
            let quote = quote(row);
            // The generator marks a row overflowing when either the quote or either split reverts.
            let _ = compute_split(
                quote.effective_floor,
                quote.price,
                true,
                row.payout_bps,
                row.protocol_bps,
            );
            let _ = compute_split(
                quote.effective_floor,
                quote.price,
                false,
                row.payout_bps,
                row.protocol_bps,
            );
        }))
        .is_err();

        assert!(
            reverted,
            "port returned a value where Solidity reverted — {}",
            describe(row)
        );
    }
}

// V2 grid

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum V2Row {
    #[serde(rename = "ask")]
    Ask(AskRow),
    #[serde(rename = "rent")]
    Rent(RentRow),
    #[serde(rename = "fee")]
    Fee(FeeRow),
}

#[derive(Debug, Deserialize)]
struct AskRow {
    ask_floor: String,
    last_price: String,
    base_price: String,
    max_ask_bps: u64,
    overflow: bool,
    reversion_base: Option<String>,
    ask_ceiling: Option<String>,
    price: Option<String>,
    effective_floor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RentRow {
    prepaid: String,
    start: String,
    expiry: String,
    now_ts: String,
    overflow: bool,
    accrued: Option<String>,
    unaccrued: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeeRow {
    rate_per_day: String,
    duration_secs: String,
    protocol_rent_bps: u64,
    fee_bps: u64,
    overflow: bool,
    cost: Option<String>,
    protocol_cut: Option<String>,
    site_cut: Option<String>,
    net: Option<String>,
}

/// The ask grid holds the pricing config fixed to the Standard profile, exactly like `_emitAskRow`.
const ASK_GRID_ELAPSED_WEEKS: u64 = 3;
const ASK_GRID_DECAY_BPS: u64 = 9_700;
const ASK_GRID_TAKE_BPS: u64 = 14_000;
const ASK_GRID_MAX_DECAY_WEEKS: u64 = 52;

fn ask_quote(row: &AskRow) -> pricing::TakeQuote {
    compute_take_price(
        pricing::resolve_reversion_base(dec(&row.ask_floor), dec(&row.last_price)),
        dec(&row.base_price),
        ASK_GRID_ELAPSED_WEEKS,
        ASK_GRID_DECAY_BPS,
        ASK_GRID_TAKE_BPS,
        ASK_GRID_MAX_DECAY_WEEKS,
    )
}

fn describe_ask(row: &AskRow) -> String {
    format!(
        "ask={} last={} base={} maxAskBps={}",
        row.ask_floor, row.last_price, row.base_price, row.max_ask_bps
    )
}

fn describe_rent(row: &RentRow) -> String {
    format!(
        "prepaid={} start={} expiry={} now={}",
        row.prepaid, row.start, row.expiry, row.now_ts
    )
}

fn describe_fee(row: &FeeRow) -> String {
    format!(
        "rate={} duration={} protocol={} fee={}",
        row.rate_per_day, row.duration_secs, row.protocol_rent_bps, row.fee_bps
    )
}

fn v2_rows() -> (Vec<AskRow>, Vec<RentRow>, Vec<FeeRow>) {
    let rows: Vec<V2Row> = read_jsonl("v2-vectors.jsonl");
    let mut asks = Vec::new();
    let mut rents = Vec::new();
    let mut fees = Vec::new();
    for row in rows {
        match row {
            V2Row::Ask(r) => asks.push(r),
            V2Row::Rent(r) => rents.push(r),
            V2Row::Fee(r) => fees.push(r),
        }
    }
    (asks, rents, fees)
}

#[test]
fn v2_grid_covers_all_three_families_and_their_boundaries() {
    let (asks, rents, fees) = v2_rows();
    assert!(asks.len() > 100, "ask family too small: {}", asks.len());
    assert!(rents.len() > 100, "rent family too small: {}", rents.len());
    assert!(fees.len() > 50, "fee family too small: {}", fees.len());

    assert!(
        asks.iter().any(|r| r.ask_floor == "0"),
        "askFloor==0 fall-through missing"
    );
    assert!(
        asks.iter().any(|r| r.max_ask_bps == 10_000),
        "maxAskBps floor missing"
    );
    assert!(
        rents.iter().any(|r| r.expiry == "0"),
        "no-tenancy row missing"
    );
    assert!(
        rents
            .iter()
            .any(|r| r.expiry != "0" && dec(&r.now_ts) < dec(&r.expiry)),
        "pre-expiry row missing"
    );
    assert!(
        rents
            .iter()
            .any(|r| r.expiry != "0" && dec(&r.now_ts) > dec(&r.expiry)),
        "post-expiry row missing"
    );
    assert!(
        fees.iter()
            .any(|r| r.protocol_rent_bps + r.fee_bps == 1_000),
        "rent band lower end missing"
    );
    assert!(
        fees.iter()
            .any(|r| r.protocol_rent_bps + r.fee_bps == 4_000),
        "rent band upper end missing"
    );
    assert!(fees.iter().any(|r| r.fee_bps == 0), "zero site fee missing");
    assert!(
        fees.iter().any(|r| r.protocol_rent_bps == 0),
        "zero protocol rent missing"
    );
    assert!(
        fees.iter().any(|r| r.cost.as_deref() == Some("0")),
        "zero-cost dust row missing"
    );

    for (name, count, overflow, computable) in [
        (
            "ask",
            asks.len(),
            asks.iter().filter(|r| r.overflow).count(),
            asks.iter().filter(|r| !r.overflow).count(),
        ),
        (
            "rent",
            rents.len(),
            rents.iter().filter(|r| r.overflow).count(),
            rents.iter().filter(|r| !r.overflow).count(),
        ),
        (
            "fee",
            fees.len(),
            fees.iter().filter(|r| r.overflow).count(),
            fees.iter().filter(|r| !r.overflow).count(),
        ),
    ] {
        assert!(count > 0, "{name} family empty");
        assert!(overflow > 0, "{name} has no overflow rows");
        assert!(computable > 0, "{name} has no computable rows");
    }
}

#[test]
fn ask_rows_match_the_reversion_base_ceiling_and_quote() {
    let (asks, _, _) = v2_rows();

    for row in asks.iter().filter(|r| !r.overflow) {
        let base = pricing::resolve_reversion_base(dec(&row.ask_floor), dec(&row.last_price));
        let ceiling =
            pricing::ask_ceiling(dec(&row.last_price), dec(&row.base_price), row.max_ask_bps);
        assert_eq!(
            base.to_string(),
            row.reversion_base.as_deref().unwrap(),
            "base — {}",
            describe_ask(row)
        );
        assert_eq!(
            ceiling.to_string(),
            row.ask_ceiling.as_deref().unwrap(),
            "ceiling — {}",
            describe_ask(row)
        );

        let quote = ask_quote(row);
        assert_eq!(
            quote.price.to_string(),
            row.price.as_deref().unwrap(),
            "price — {}",
            describe_ask(row)
        );
        assert_eq!(
            quote.effective_floor.to_string(),
            row.effective_floor.as_deref().unwrap(),
            "floor — {}",
            describe_ask(row)
        );
    }
}

/// §3.1: no posted ask must price exactly like the same inputs with no ask dimension at all.
#[test]
fn unset_ask_leaves_pricing_untouched() {
    let (asks, _, _) = v2_rows();
    let unset: Vec<&AskRow> = asks
        .iter()
        .filter(|r| r.ask_floor == "0" && !r.overflow)
        .collect();
    assert!(!unset.is_empty(), "no unset-ask rows");

    for row in unset {
        let without_ask = compute_take_price(
            dec(&row.last_price),
            dec(&row.base_price),
            ASK_GRID_ELAPSED_WEEKS,
            ASK_GRID_DECAY_BPS,
            ASK_GRID_TAKE_BPS,
            ASK_GRID_MAX_DECAY_WEEKS,
        );
        assert_eq!(
            without_ask.price.to_string(),
            row.price.as_deref().unwrap(),
            "price changed by a zero ask — {}",
            describe_ask(row)
        );
        assert_eq!(
            without_ask.effective_floor.to_string(),
            row.effective_floor.as_deref().unwrap(),
            "floor changed by a zero ask — {}",
            describe_ask(row)
        );
    }
}

#[test]
fn rent_accrual_matches_both_legs() {
    let (_, rents, _) = v2_rows();

    for row in rents.iter().filter(|r| !r.overflow) {
        let prepaid = dec(&row.prepaid);
        let start = num(&row.start);
        let expiry = num(&row.expiry);
        let now = num(&row.now_ts);

        let accrued = rentals::accrued_of(prepaid, start, expiry, now);
        let both = accrual(prepaid, start, expiry, now);

        assert_eq!(
            accrued.to_string(),
            row.accrued.as_deref().unwrap(),
            "accrued — {}",
            describe_rent(row)
        );
        assert_eq!(
            both.accrued,
            accrued,
            "accrual/accrued_of disagree — {}",
            describe_rent(row)
        );
        assert_eq!(
            both.unaccrued.to_string(),
            row.unaccrued.as_deref().unwrap(),
            "unaccrued — {}",
            describe_rent(row)
        );
        assert_eq!(
            both.accrued + both.unaccrued,
            prepaid,
            "legs do not sum to prepaid — {}",
            describe_rent(row)
        );
    }
}

#[test]
fn fee_rows_match_cost_and_split() {
    let (_, _, fees) = v2_rows();

    for row in fees.iter().filter(|r| !r.overflow) {
        let cost = rent_cost(dec(&row.rate_per_day), num(&row.duration_secs));
        assert_eq!(
            cost.to_string(),
            row.cost.as_deref().unwrap(),
            "cost — {}",
            describe_fee(row)
        );

        let split = rent_split(cost, row.protocol_rent_bps, row.fee_bps);
        assert_eq!(
            split.protocol_cut.to_string(),
            row.protocol_cut.as_deref().unwrap(),
            "protocol cut — {}",
            describe_fee(row)
        );
        assert_eq!(
            split.site_cut.to_string(),
            row.site_cut.as_deref().unwrap(),
            "site cut — {}",
            describe_fee(row)
        );
        assert_eq!(
            split.net.to_string(),
            row.net.as_deref().unwrap(),
            "net — {}",
            describe_fee(row)
        );
        assert_eq!(
            split.protocol_cut + split.site_cut + split.net,
            cost,
            "legs do not sum to cost — {}",
            describe_fee(row)
        );
    }
}

#[test]
fn v2_overflow_rows_revert_exactly_where_solidity_does() {
    let (asks, rents, fees) = v2_rows();

    for row in asks.iter().filter(|r| r.overflow) {
        let reverted = catch_unwind(AssertUnwindSafe(|| {
            let _ =
                pricing::ask_ceiling(dec(&row.last_price), dec(&row.base_price), row.max_ask_bps);
            let _ = ask_quote(row);
        }))
        .is_err();
        assert!(reverted, "ask row did not revert — {}", describe_ask(row));
    }

    for row in rents.iter().filter(|r| r.overflow) {
        let prepaid = dec(&row.prepaid);
        let start = num(&row.start);
        let expiry = num(&row.expiry);
        let now = num(&row.now_ts);
        let reverted = catch_unwind(AssertUnwindSafe(|| {
            let _ = accrual(prepaid, start, expiry, now);
        }))
        .is_err();
        assert!(reverted, "rent row did not revert — {}", describe_rent(row));
    }

    for row in fees.iter().filter(|r| r.overflow) {
        let reverted = catch_unwind(AssertUnwindSafe(|| {
            let cost = rent_cost(dec(&row.rate_per_day), num(&row.duration_secs));
            let _ = rent_split(cost, row.protocol_rent_bps, row.fee_bps);
        }))
        .is_err();
        assert!(reverted, "fee row did not revert — {}", describe_fee(row));
    }
}
