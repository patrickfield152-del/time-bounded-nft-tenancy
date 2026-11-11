//! `SiteTransferHook`: revoke every approval for a token in its pre-`f`
//! block, and nothing else.
//!
//! Why the ordering is load-bearing (`ExcessiveUnlockError`):
//!
//! `#[derive(NonFungibleToken)]` nests the transfer hooks as
//! `(user_transfer_hook, (user_all_hooks, (Nep171StorageAccountingHook,
//! (TokenApprovals, TokenEnumeration))))`, left side outermost. The accounting
//! hook snapshots `env::storage_usage()` *before* the inner hooks run, then
//! charges or credits the receiver for the net change seen up to its
//! post-`f` step. `TokenApprovals`' own post-`f` step calls
//! `revoke_all_unchecked`, which frees the approval storage inside that
//! window: the accounting hook then reads a net storage *release* and calls
//! `unlock_storage(receiver, released_bytes)`.
//!
//! For a receiver that is being registered at exactly `bounds.min` (a fresh
//! buyer funded with the shortfall), that credit can exceed their `total`
//! balance and `unlock_storage` panics with `ExcessiveUnlockError`
//! (nearsea PR #19). Revoking here, before `storage_usage_start` is taken,
//! moves the write outside the accounting window, so the window sees no
//! approval delta and the mint/transfer proceeds normally.
//!
//! The hook changes nothing else: a plain `nft_transfer` (including the escrow
//! vault's surrender) changes ownership only, matching the origin contract,
//! where settlement and listing-clear live in `_buy` alone.

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
