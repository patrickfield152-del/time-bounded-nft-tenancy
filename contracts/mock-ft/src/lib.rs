//! stele mock settlement token: NEP-141/145/148 with 6 decimals (`FungibleToken` derive from `near-sdk-contract-tools` 4.0.0 generates `ft_transfer`, `ft_transfer_call`, `ft_balance_of`, `ft_total_supply`, `ft_metadata`, `storage_deposit`, `storage_withdraw`, `storage_unregister`, `storage_balance_of`).
//! Factory derives `min_slot_floor = 10^(6 - 4) = 100` from [`DECIMALS`] (`24` for native NEAR; `1_000_000` = 1 token); `storage_deposit` requires 10 mNEAR.

use near_sdk::json_types::U128;
use near_sdk::{AccountId, NearToken, PanicOnDefault, env, near, require};
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
    /// Sets `owner` and NEP-148 metadata (`Nep148Controller::set_metadata`, `ContractMetadata::new` `(name, symbol, decimals)`).
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

    /// Owner-only mint (`ft_mint`); receiver must be NEP-145 registered.
    pub fn mint(&mut self, account_id: AccountId, amount: U128) {
        self.assert_owner();
        let mint = Nep141Mint::new(amount.0, account_id);
        Nep141Controller::mint(self, &mint)
            .unwrap_or_else(|error| env::panic_str(&format!("MintFailed: {error}")));
    }

    /// Owner-only burn (`ft_burn`).
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
        require!(env::predecessor_account_id() == self.owner, "NotOwner");
    }
}
