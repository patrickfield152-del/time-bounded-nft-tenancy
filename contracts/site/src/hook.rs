//! `SiteTransferHook`: revokes token approvals via `revoke_all_unchecked` in pre-`f` before `Nep171StorageAccountingHook` inside `#[derive(NonFungibleToken)]` (`(user_transfer_hook, (user_all_hooks, (Nep171StorageAccountingHook, (TokenApprovals, TokenEnumeration))))`) snapshots `storage_usage_start` (`env::storage_usage()`).
//! Post-`f` revocation lowers storage and triggers `unlock_storage(receiver, released_bytes)` (`ExcessiveUnlockError`, exceeding `total` at `bounds.min`, nearsea PR #19); plain `nft_transfer` leaves settlement in `_buy`.

use near_sdk_contract_tools::hook::Hook;
use near_sdk_contract_tools::nft::{Nep171Transfer, Nep178Controller};

use crate::Contract;

/// Revokes approvals for the transferred token before the transfer body runs.
pub struct SiteTransferHook;

impl Hook<Contract, Nep171Transfer<'_>> for SiteTransferHook {
    fn hook<R>(
        contract: &mut Contract,
        transfer: &Nep171Transfer<'_>,
        f: impl FnOnce(&mut Contract) -> R,
    ) -> R {
        contract.revoke_all_unchecked(&transfer.token_id);
        f(contract)
    }
}
