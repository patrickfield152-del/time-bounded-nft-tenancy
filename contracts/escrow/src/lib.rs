//! stele escrow vault: balance-delta booking, dark clock, and the three-phase
//! NEP-178 surrender claim.
//!
//! Wave 1 (REPO) fixes the state surface transcribed from `p0-nfts.md`
//! sections 3.5 and 3.6. `bind`, `book`, `release`, `claim`, `mark_dark`,
//! `clear_dark`, the reserve/withdraw ledger and the views land in wave 3.

use near_sdk::json_types::U128;
use near_sdk::store::{LookupMap, Vector};
use near_sdk::{near, AccountId};

pub type TokenId = String;

/// Factory-supplied vault parameters, immutable after `bind`.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscrowParams {
    pub window_secs: u64,
    pub claim_delay_secs: u64,
    pub min_transition_secs: u64,
    pub min_booking: U128,
}

/// One booked value arrival. `dark_at_booking` freezes maturity as of the
/// booking moment, mirroring `_mature` evaluated AS OF `dark_since`.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deposit {
    pub amount: U128,
    pub booked_at: u64,
    pub dark_at_booking: u64,
    pub beneficiary: AccountId,
}

#[near(contract_state)]
#[derive(near_sdk::PanicOnDefault)]
pub struct Contract {
    pub factory: AccountId,
    pub site: Option<AccountId>,
    pub settlement_ft: Option<AccountId>,
    pub window_secs: u64,
    pub claim_delay_secs: u64,
    pub min_transition_secs: u64,
    pub min_booking: U128,
    /// Nanoseconds; the dark clock is the maturity reference.
    pub dark_since: u64,
    pub dark_accrued: u64,
    pub last_transition_at: u64,
    pub booked_total: U128,
    pub pending_total: U128,
    pub reserve: U128,
    pub pending: LookupMap<AccountId, U128>,
    /// One entry per token per dark episode; rollover removes it.
    pub claimed_epoch: LookupMap<TokenId, u64>,
    /// `(token_id, owner)` -> approval id captured by `nft_on_approve`.
    pub approvals: LookupMap<(TokenId, AccountId), u64>,
    /// Site legs cached at book time (`claimable_local` is cache-only).
    pub site_treasury_balance: U128,
    pub site_pending: U128,
    pub deposits: Vector<Deposit>,
    pub head: u64,
}
