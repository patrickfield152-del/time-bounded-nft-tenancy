//! stele mock settlement token: NEP-141/145/148 with 6 decimals.
//!
//! Port of `mocks/TestUSD.sol`. The `FungibleToken` derive (near-sdk-contract-tools
//! 4.0.0) expands to NEP-141 + NEP-145 + NEP-148, so `ft_transfer`,
//! `ft_transfer_call`, `ft_balance_of`, `ft_total_supply`, `ft_metadata`,
//! `storage_deposit`, `storage_withdraw`, `storage_unregister` and
//! `storage_balance_of` are all generated. Only the owner-mint and owner-burn
//! levers are written by hand here.
//!
//! Decimals are 6, not 24: the factory derives
//! `min_slot_floor = 10^(6 - 4) = 100` from [`DECIMALS`], so a floor of
//! `1_000_000` is one whole token and the demo economics read in cents.
//!
//! Because NEP-145 is integrated, minting and transferring require the
//! receiver to be registered (`storage_deposit`). That is the factory's job for
//! the vault, and the buyer's job for a personal balance; the NEP-145 check is
//! what makes a token balance a paid-for storage slot rather than free state.

use near_sdk::json_types::U128;
use near_sdk::{env, near, require, AccountId, NearToken, PanicOnDefault};
use near_sdk_contract_tools::ft::{
    ContractMetadata, FungibleToken, Nep141Burn, Nep141Controller, Nep141Mint, Nep145Controller,
    Nep148Controller, StorageBalanceBounds,
};

/// Mock settlement asset precision; the factory derives
/// `min_slot_floor = 10^(6 - 4) = 100` from it.
pub const DECIMALS: u8 = 6;
/// NEP-148 token name.
pub const NAME: &str = "stele mock USD";
/// NEP-148 token symbol.
pub const SYMBOL: &str = "steleUSD";

#[near(contract_state)]
#[derive(PanicOnDefault, FungibleToken)]
pub struct Contract {
    /// The mint/burn authority. Deliberately a plain state field: the FT has no
    /// admin methods beyond mint and burn, so the `Owner` derive's
    /// transfer/propose/renounce surface would be dead weight.
    pub owner: AccountId,
}

#[near]
impl Contract {
    /// Sets the owner slot and the NEP-148 metadata. The derive has no
    /// constructor of its own: in 4.0.0 metadata is written through
    /// `Nep148Controller::set_metadata` (the crate's own derive test does the
    /// same), and `ContractMetadata::new` takes `(name, symbol, decimals)`.
    #[init]
    pub fn new(owner_id: AccountId) -> Self {
        let mut contract = Self { owner: owner_id };
        contract.set_metadata(&ContractMetadata::new(NAME, SYMBOL, DECIMALS));
        // NEP-145's default bound is zero, which would make
        // `storage_deposit(registration_only = true)` register a balance too
        // small to hold a token. Ten mNEAR is the asset's real per-token stake.
        contract.set_storage_balance_bounds(&StorageBalanceBounds {
            min: NearToken::from_millinear(10),
            max: None,
        });
        contract
    }

    /// Owner-only mint, event-emitting (`ft_mint` via the NEP-141 controller).
    /// The receiver must be NEP-145-registered: the derive's storage
    /// accounting hook runs on the mint path, so minting to a fresh account
    /// panics with a registration/balance error rather than gifting state.
    pub fn mint(&mut self, account_id: AccountId, amount: U128) {
        self.assert_owner();
        let mint = Nep141Mint::new(amount.0, account_id);
        Nep141Controller::mint(self, &mint)
            .unwrap_or_else(|error| env::panic_str(&format!("MintFailed: {error}")));
    }

    /// Owner-only burn, event-emitting (`ft_burn`). Burns from the named
    /// account, which is what lets the demo reset a balance without a
    /// transfer; an underflow panics.
    pub fn burn(&mut self, account_id: AccountId, amount: U128) {
        self.assert_owner();
        let burn = Nep141Burn::new(amount.0, account_id);
        Nep141Controller::burn(self, &burn)
            .unwrap_or_else(|error| env::panic_str(&format!("BurnFailed: {error}")));
    }
}

impl Contract {
    /// The one admin gate. Panics for anyone but the initialized owner.
    fn assert_owner(&self) {
        require!(
            env::predecessor_account_id() == self.owner,
            "NotOwner"
        );
    }
}
