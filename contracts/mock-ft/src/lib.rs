//! stele mock settlement token: NEP-141/145/148 with 6 decimals.
//!
//! Wave 1 (REPO) declares the owner slot only. The `FungibleToken` derive
//! (Nep141+Nep145+Nep148), `ft_metadata`, owner-mint and storage methods land
//! in wave 3, step 10 of the spec.

use near_sdk::{near, AccountId};

/// Mock settlement asset precision; the factory derives
/// `min_slot_floor = 10^(6 - 4) = 100` from it.
pub const DECIMALS: u8 = 6;

#[near(contract_state)]
#[derive(near_sdk::PanicOnDefault)]
pub struct Contract {
    pub owner: AccountId,
}
