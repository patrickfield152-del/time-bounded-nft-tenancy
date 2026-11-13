//! Additive bundle view and encumbrance parity.
//!
//! `slot_bundle` must return exactly the standalone views' values, and the
//! encumbrance preimage must match the cross-language fixture the SDK hashes
//! locally. The fixture lives in `vectors/encumbrance-vectors.jsonl`; a missing
//! row is a failure, never a skip.

use near_sdk::json_types::U128;
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{testing_env, AccountId};
use serde::Deserialize;
use stele_site::views::encumbrance_preimage;
use stele_site::{Contract, Listing, Quote, Rental, SiteConfig, Slot};

fn account(id: &str) -> AccountId {
    id.parse().expect("valid test account")
}

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

fn contract() -> Contract {
    testing_env!(VMContextBuilder::new().build());
    Contract::new(
        account("owner.test.near"),
        account("protocol.test.near"),
        500,
        500,
        None,
        None,
        false,
        config(),
        Vec::new(),
        Vec::new(),
        U128(1),
    )
}

fn slot(base_price: u128, registered: bool) -> Slot {
    Slot {
        last_price: U128(0),
        base_price: U128(base_price),
        page_hash: String::new(),
        ask_floor: U128(0),
        last_purchase_at_ns: 0,
        floor_updated_at_ns: 0,
        content_version: 0,
        takes: 0,
        registered,
        available: false,
    }
}

fn rental(tenant: &str, expiry: u64, prepaid: u128, claimed: u128) -> Rental {
    Rental {
        tenant: account(tenant),
        start: 0,
        expiry,
        prepaid: U128(prepaid),
        claimed: U128(claimed),
        owner_hash_snapshot: String::new(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn slot_bundle_matches_standalone_views() {
    let mut c = contract();
    let id = "hero.headline".to_owned();
    let slot = slot(1_000, true);
    let listing = Listing {
        rate_per_day: U128(50),
        max_duration_secs: 86_400,
        fee_bps: 250,
    };
    let rental = rental("tenant.test.near", 1_800_000_000_000, 900, 100);
    c.slots.insert(id.clone(), slot.clone());
    c.listings.insert(id.clone(), listing.clone());
    c.rentals.insert(id.clone(), rental.clone());

    let bundle = c.slot_bundle(id.clone());
    assert_eq!(bundle.slot.as_ref(), Some(&slot));
    assert_eq!(bundle.quote, c.quote(id.clone()));
    assert_eq!(bundle.owner, c.owner_of_or_zero(id.clone()));
    assert_eq!(bundle.listing.as_ref(), Some(&listing));
    assert_eq!(bundle.rental.as_ref(), Some(&rental));

    let empty = c.slot_bundle("never.registered".to_owned());
    assert_eq!(empty.slot, None);
    assert_eq!(empty.quote, Quote { effective_floor: U128(0), price: U128(0) });
    assert_eq!(empty.owner, None);
    assert_eq!(empty.listing, None);
    assert_eq!(empty.rental, None);
}

#[derive(Deserialize)]
struct FixtureSlot {
    base_price: String,
    registered: bool,
}

#[derive(Deserialize)]
struct FixtureRental {
    tenant: String,
    expiry: String,
    prepaid: String,
    claimed: String,
}

#[derive(Deserialize)]
struct FixtureRow {
    name: String,
    token_id: String,
    slot: Option<FixtureSlot>,
    rental: Option<FixtureRental>,
    preimage_hex: String,
    hash: String,
}

#[test]
fn encumbrance_matches_cross_language_vectors() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vectors/encumbrance-vectors.jsonl");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing fixture {} ({e})", path.display()));
    let rows: Vec<FixtureRow> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("bad fixture row: {e}")))
        .collect();
    assert!(!rows.is_empty(), "encumbrance fixture is empty");

    let mut c = contract();
    for row in rows {
        let id = row.token_id.clone();
        if let Some(fixture) = &row.slot {
            c.slots.insert(
                id.clone(),
                slot(fixture.base_price.parse().expect("u128 base price"), fixture.registered),
            );
        }
        if let Some(fixture) = &row.rental {
            c.rentals.insert(
                id.clone(),
                rental(
                    &fixture.tenant,
                    fixture.expiry.parse().expect("u64 expiry"),
                    fixture.prepaid.parse().expect("u128 prepaid"),
                    fixture.claimed.parse().expect("u128 claimed"),
                ),
            );
        }

        let preimage = encumbrance_preimage(c.slots.get(&id), c.rentals.get(&id));
        assert_eq!(hex(&preimage), row.preimage_hex, "preimage drift for {}", row.name);
        assert_eq!(c.encumbrance_hash(id), row.hash, "hash drift for {}", row.name);
    }
}
