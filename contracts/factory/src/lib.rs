//! stele factory: per-site sub-account deployment, escrow-vault wiring, and
//! the rotatable attestor registry.
//!
//! Wave 1 (REPO) fixes the state surface only. `create_site`, `create_site_for`,
//! `create_escrowed_site`, `set_escrow_params`, `allow_token`, `vault_of`,
//! `set_attestor`, `mark_dark`/`clear_dark` and `get_required` land in waves 2
//! and 3. Site and escrow wasm are embedded at wave 3; this crate stays
//! declaration-only until then.

use near_sdk::json_types::U128;
use near_sdk::store::LookupMap;
use near_sdk::{near, AccountId};

/// Attestor pointer; `None` means fail-open, as in the origin registry.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactoryParams {
    pub protocol_treasury: AccountId,
    pub protocol_bps: u64,
    pub protocol_rent_bps: u64,
    pub attestor: Option<AccountId>,
}

/// Settlement-asset allow-list entry. `decimals` is what the factory uses to
/// derive `min_slot_floor = 10^(decimals - 4)` for every site it creates
/// (native NEAR is 24 decimals; the mock FT is 6).
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowedToken {
    pub decimals: u8,
    pub min_slot_floor: U128,
}

#[near(contract_state)]
#[derive(near_sdk::PanicOnDefault)]
pub struct Contract {
    pub protocol_treasury: AccountId,
    pub protocol_bps: u64,
    pub protocol_rent_bps: u64,
    pub attestor: Option<AccountId>,
    /// Settlement asset -> decimals/floor policy. Native NEAR is keyed by the
    /// `near` pseudo-account id so the same map answers both branches.
    pub allowed_tokens: LookupMap<AccountId, AllowedToken>,
    /// Site account -> escrow vault account.
    pub vault_of: LookupMap<AccountId, AccountId>,
}
