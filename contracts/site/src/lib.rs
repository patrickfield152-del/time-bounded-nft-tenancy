//! stele site contract: NEP-171 slot NFTs, the market spine, and the pull
//! ledger.
//! Wave 2 builds the state surface and money spine from `p0-nfts.md`
//! sections 3.3, 3.6 and 4 steps 5-6. `NonFungibleToken` derives
//! NEP-171/177/178/181 and NEP-145; `Owner` holds the admin slot, with no
//! `owner_id` field. This crate adds registration, the floor/ask levers,
//! `buy`/`buy_for` with the storage gate, the pull ledger and admin setters.
//! Tenancy (`list_for_rent`, `rent`, `extend_rental`, `claim_rent`,
//! `end_rental`, `edit`, `can_edit`, `set_editor`, `editor_of`) lives here.
//! A sale settles the outgoing rent, clears the listing, and keeps the term.
//! Modules: [`pricing`] pure `U256` money math (the parity target);
//! [`rentals`] pure rent math and predicates; [`terms`] clamps and ratchets;
//! [`events`] the 39 ported NEP-297 events; [`transfer_hook`] approval
//! revocation; [`views`] every read-only method; [`ft_receiver`] the NEP-141
//! `ft_on_transfer` entry point.

pub mod events;
pub mod ft_receiver;
pub mod pricing;
pub mod rentals;
pub mod terms;
pub mod transfer_hook;
pub mod views;

pub use transfer_hook::SiteTransferHook;

use std::borrow::Cow;

use near_sdk::borsh::BorshSerialize;
use near_sdk::json_types::{U128, U64};
use near_sdk::store::{LookupMap, Vector};
use near_sdk::utils::{assert_one_yocto, is_promise_success};
use near_sdk::{
    env, near, require, AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault, Promise,
    PromiseOrValue,
};
use near_sdk_contract_tools::nft::nep171::Nep171TransferAuthorization;
use near_sdk_contract_tools::nft::{
    ext_nep145, ContractMetadata, Nep145Controller, Nep171Controller, Nep171Transfer,
    Nep177Controller, Nep178Controller, StorageBalanceBounds, Token, TokenMetadata,
};
use near_sdk_contract_tools::owner::Owner as OwnerTrait;
use near_sdk_contract_tools::{NonFungibleToken, Owner};
use primitive_types::U256;

use crate::events::SteleEvent;
use crate::pricing::TakeQuote;

/// Nanoseconds per second: configuration stays in seconds, state keeps
/// nanoseconds, and every comparison converts here.
pub(crate) const NS_PER_SEC: u64 = 1_000_000_000;
/// Nanoseconds per millisecond: NEP-177 timestamps are milliseconds.
pub(crate) const NS_PER_MS: u64 = 1_000_000;
/// NEP-145 minimum balance, also the port's stand-in for one token's storage
/// bytes: 700 bytes at 1e19 yocto per byte is 7 mNEAR (the tutorial bound).
pub(crate) const TOKEN_STORAGE_MILLINEAR: u128 = 7;
/// NEP-145 maximum balance (the tutorial bound, 21 mNEAR).
pub(crate) const MAX_STORAGE_MILLINEAR: u128 = 21;
/// `MAX_ROYALTY_BPS` from the origin.
pub(crate) const MAX_ROYALTY_BPS: u16 = 1_000;
/// The origin `implementationVersion()` value.
pub(crate) const IMPLEMENTATION_VERSION: u32 = 2;

/// Readable dotted slot key, used directly as the NEP-171 `token_id`
/// (documented deviation: origin used a hashed key).
pub type TokenId = String;

/// Per-slot market state. Timestamps are nanoseconds at the contract boundary;
/// pure math in `pricing`/`rentals` stays in seconds.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub last_price: U128,
    pub base_price: U128,
    pub page_hash: String,
    pub ask_floor: U128,
    pub last_purchase_at_ns: u64,
    pub floor_updated_at_ns: u64,
    pub content_version: u32,
    pub takes: u32,
    pub registered: bool,
    pub available: bool,
}

/// Immutable-per-site economics validated by `terms::check_economics_absolute`.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteConfig {
    pub take_bps: u64,
    pub payout_bps: u64,
    pub reversion_bps: u64,
    pub max_reversion_weeks: u32,
    pub cooldown_secs: u64,
    pub floor_delta_bps: u64,
    pub floor_change_cooldown_secs: u64,
    pub max_ask_bps: u64,
    pub min_rent_bps: u64,
    pub site_rent_bps: u64,
    pub max_rental_term_secs: u64,
}

/// Effective floor and take price for one slot.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quote {
    pub effective_floor: U128,
    pub price: U128,
}

/// Tenancy listing; `fee_bps` is snapshotted when the listing is created.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub rate_per_day: U128,
    pub max_duration_secs: u64,
    pub fee_bps: u16,
}

/// Live tenancy. `start`/`expiry` are nanoseconds; `owner_hash_snapshot` is
/// written only by `open` and restored by `finish`.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rental {
    pub tenant: AccountId,
    pub start: u64,
    pub expiry: u64,
    pub prepaid: U128,
    pub claimed: U128,
    pub owner_hash_snapshot: String,
}

/// Editor grant; every take invalidates it via `granted_at_takes`.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorGrant {
    pub editor: AccountId,
    pub grantor: AccountId,
    pub granted_at_takes: u32,
}

/// The settled economics of one purchase, computed in the planning receipt and
/// replayed by `settle_buy_callback` so a storage top-up cannot change them.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuyPlan {
    pub price: U128,
    pub payout: U128,
    pub protocol_cut: U128,
    pub site_cut: U128,
    /// `None` = the slot has never been claimed, so the sale mints.
    pub previous_owner: Option<AccountId>,
}

/// Storage prefixes for the site-owned maps. Distinct from the derive's
/// `DefaultStorageKey` prefixes (`~$171`, `~$177`, ...), which begin with
/// ASCII bytes.
#[derive(BorshSerialize, BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
enum StorageKey {
    Slots,
    Registered,
    Listings,
    Rentals,
    EditorGrants,
    PendingWithdrawals,
}

#[near(contract_state)]
#[derive(PanicOnDefault, Owner, NonFungibleToken)]
#[non_fungible_token(transfer_hook = "SiteTransferHook")]
pub struct Contract {
    /// Owner lives in the `Owner` derive slot; there is no `owner_id` field.
    pub protocol_treasury: AccountId,
    pub protocol_bps: u64,
    pub protocol_rent_bps: u64,
    /// `None` = native NEAR settlement.
    pub settlement_ft: Option<AccountId>,
    pub config: SiteConfig,
    pub terms_locked: bool,
    pub paused: bool,
    pub open_registration: bool,
    pub treasury: AccountId,
    pub treasury_pinned: bool,
    pub default_floor: U128,
    /// Factory-supplied: `10^(settlement decimals - 4)`.
    pub min_slot_floor: U128,
    pub treasury_royalty_bps: u16,
    /// Starts at 2 and only changes through a state migration.
    pub impl_version: u32,
    pub slots: LookupMap<TokenId, Slot>,
    pub registered: Vector<TokenId>,
    pub listings: LookupMap<TokenId, Listing>,
    pub rentals: LookupMap<TokenId, Rental>,
    pub total_escrowed_rent: U128,
    pub editor_grants: LookupMap<TokenId, EditorGrant>,
    pub pending_withdrawals: LookupMap<AccountId, U128>,
    pub treasury_balance: U128,
}

/// External surface: construction, registration, the floor/ask levers, the
/// market spine, the pull ledger and the admin setters.
#[near]
impl Contract {
    /// Origin `initialize`, plus the two factory-supplied treasury fields.
    /// The factory supplies `min_slot_floor` from the settlement asset's real
    /// decimals: native 24 gives `10^20`; the 6-decimal mock FT gives `100`.
    /// `new` does not read `ft_metadata`, because that call is asynchronous.
    /// Documented deviation: the origin took a treasury address that the owner
    /// could repoint until a factory pinned it later. On NEAR the factory
    /// creates an escrowed site whose `Owner` derive slot is the publisher, not
    /// the factory, so a post-create owner call cannot pin the treasury in one
    /// atomic step. `new` therefore takes `treasury` (default:
    /// `protocol_treasury`) and `treasury_pinned`, so `create_escrowed_site`
    /// can pin the vault in the same receipt as the deploy. A pinned site's
    /// `set_treasury` refuses to repoint.
    #[init]
    pub fn new(
        owner_id: AccountId,
        protocol_treasury: AccountId,
        protocol_bps: u64,
        protocol_rent_bps: u64,
        settlement_ft: Option<AccountId>,
        treasury: Option<AccountId>,
        treasury_pinned: bool,
        config: SiteConfig,
        keys: Vec<TokenId>,
        floors: Vec<U128>,
        min_slot_floor: U128,
    ) -> Self {
        terms::check_economics_absolute(&config, protocol_bps, protocol_rent_bps);
        terms::check_config_absolute(&config);
        require!(min_slot_floor.0 != 0, "InvalidFloor");

        let treasury = treasury.unwrap_or_else(|| protocol_treasury.clone());
        let mut contract = Self {
            protocol_treasury,
            protocol_bps,
            protocol_rent_bps,
            settlement_ft,
            config,
            terms_locked: false,
            paused: false,
            open_registration: false,
            treasury,
            treasury_pinned,
            default_floor: min_slot_floor,
            min_slot_floor,
            treasury_royalty_bps: 0,
            impl_version: IMPLEMENTATION_VERSION,
            slots: LookupMap::new(StorageKey::Slots),
            registered: Vector::new(StorageKey::Registered),
            listings: LookupMap::new(StorageKey::Listings),
            rentals: LookupMap::new(StorageKey::Rentals),
            total_escrowed_rent: U128(0),
            editor_grants: LookupMap::new(StorageKey::EditorGrants),
            pending_withdrawals: LookupMap::new(StorageKey::PendingWithdrawals),
            treasury_balance: U128(0),
        };

        OwnerTrait::init(&mut contract, &owner_id);
        contract.set_contract_metadata(&ContractMetadata::new("stele", "STELE", None));
        contract.set_storage_balance_bounds(&StorageBalanceBounds {
            min: NearToken::from_millinear(TOKEN_STORAGE_MILLINEAR),
            max: Some(NearToken::from_millinear(MAX_STORAGE_MILLINEAR)),
        });
        contract.register_slots_inner(keys, floors);
        SteleEvent::SiteInitialized {
            owner: owner_id,
            settlement_token: contract.settlement_ft.clone(),
            take_bps: contract.config.take_bps,
            payout_bps: contract.config.payout_bps,
        }
        .emit();
        contract
    }

    /// Origin `registerSlots`. Registration is permanent. Keys are identities,
    /// so a duplicate is rejected, not re-registered.
    #[payable]
    pub fn register_slots(&mut self, keys: Vec<TokenId>, floors: Vec<U128>) {
        Self::require_owner();
        assert_one_yocto();
        self.register_slots_inner(keys, floors);
    }

    /// Origin `setAvailability`: the reversible half of registration. Legal on
    /// a claimed slot. It gates claims only, never takes.
    #[payable]
    pub fn set_availability(&mut self, keys: Vec<TokenId>, available: bool) {
        Self::require_owner();
        assert_one_yocto();
        for key in keys {
            let mut slot = self
                .slots
                .get(&key)
                .cloned()
                .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));
            require!(slot.registered, "SlotNotRegistered");
            slot.available = available;
            self.slots.insert(key.clone(), slot);
            SteleEvent::AvailabilitySet { token_id: key, available }.emit();
        }
    }

    /// Origin `setFloor`: the `min_slot_floor` floor, a per-call delta band, and
    /// a cooldown. The band and the cooldown are seconds at the boundary.
    #[payable]
    pub fn set_floor(&mut self, token_id: TokenId, new_floor: U128) {
        Self::require_owner();
        assert_one_yocto();
        let mut slot = self
            .slots
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));
        require!(slot.registered, "SlotNotRegistered");
        require!(new_floor.0 >= self.min_slot_floor.0, "InvalidFloor");

        let now_ns = env::block_timestamp();
        let now_secs = now_ns / NS_PER_SEC;
        let last_secs = slot.floor_updated_at_ns / NS_PER_SEC;
        require!(
            now_secs >= last_secs.saturating_add(self.config.floor_change_cooldown_secs),
            "FloorCooldownActive"
        );

        let current = slot.base_price.0;
        let denominator = u128::from(pricing::BPS_DENOMINATOR);
        let max_delta = current
            .checked_mul(u128::from(self.config.floor_delta_bps))
            .unwrap_or_else(|| env::panic_str("MathOverflow"))
            / denominator;
        let delta = new_floor.0.abs_diff(current);
        require!(delta <= max_delta, "FloorDeltaTooLarge");

        slot.base_price = new_floor;
        slot.floor_updated_at_ns = now_ns;
        self.slots.insert(token_id.clone(), slot);
        SteleEvent::FloorSet {
            token_id,
            floor: new_floor,
            timestamp_ns: now_ns.into(),
        }
        .emit();
    }

    /// Origin `setAsk`. The ask is the reversion BASE, not a list price, and
    /// the ceiling anchors to `last_price`/`base_price` so asks cannot compound.
    /// The one-yocto guard means a leaked function-call key cannot reprice an
    /// ask: function-call keys cannot attach a deposit.
    #[payable]
    pub fn set_ask(&mut self, token_id: TokenId, ask_floor: U128) {
        assert_one_yocto();
        let owner = self
            .token_owner(&token_id)
            .unwrap_or_else(|| env::panic_str("NotSlotOwner"));
        require!(owner == env::predecessor_account_id(), "NotSlotOwner");
        let mut slot = self
            .slots
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));

        if ask_floor.0 != 0 {
            require!(ask_floor.0 >= slot.base_price.0, "AskBelowBase");
            let ceiling = pricing::ask_ceiling(
                U256::from(slot.last_price.0),
                U256::from(slot.base_price.0),
                self.config.max_ask_bps,
            );
            require!(U256::from(ask_floor.0) <= ceiling, "AskAboveCap");
        }

        slot.ask_floor = ask_floor;
        self.slots.insert(token_id.clone(), slot);
        SteleEvent::AskSet {
            token_id,
            owner: env::predecessor_account_id(),
            ask_floor,
            timestamp_ns: env::block_timestamp().into(),
        }
        .emit();
    }

    /// Origin `buy`: native settlement, recipient is the predecessor.
    #[payable]
    pub fn buy(
        &mut self,
        token_id: TokenId,
        max_price: U128,
        expected_terms: String,
        deadline_ns: u64,
    ) -> PromiseOrValue<U128> {
        let recipient = env::predecessor_account_id();
        self.settle_buy(
            recipient,
            token_id,
            max_price,
            expected_terms,
            deadline_ns,
            U128(env::attached_deposit().as_yoctonear()),
            false,
        )
    }

    /// Origin `buyFor`: the payer is the predecessor, the recipient is not.
    #[payable]
    pub fn buy_for(
        &mut self,
        recipient: AccountId,
        token_id: TokenId,
        max_price: U128,
        expected_terms: String,
        deadline_ns: u64,
    ) -> PromiseOrValue<U128> {
        self.settle_buy(
            recipient,
            token_id,
            max_price,
            expected_terms,
            deadline_ns,
            U128(env::attached_deposit().as_yoctonear()),
            false,
        )
    }

    /// Second phase of a native buy that had to fund the recipient's NEP-145
    /// account first. Nothing is booked until this receipt runs, so a failed
    /// storage deposit returns the whole charge instead of leaving a phantom
    /// sale, and an ownership race returns the price instead of minting to a
    /// buyer who no longer has a claim.
    #[private]
    pub fn settle_buy_callback(
        &mut self,
        recipient: AccountId,
        payer: AccountId,
        token_id: TokenId,
        plan: BuyPlan,
        top_up: U128,
    ) {
        if !is_promise_success() {
            // The self-call failed, so the forwarded deposit came back to this
            // contract; the payer is made whole for the price and the top-up.
            let refund = plan
                .price
                .0
                .checked_add(top_up.0)
                .unwrap_or_else(|| env::panic_str("AmountOverflow"));
            self.credit(&payer, U128(refund));
            return;
        }
        if self.token_owner(&token_id) != plan.previous_owner {
            // The storage top-up is already staked to the recipient and is not
            // reclaimable here; the purchase principal is returned in full.
            self.credit(&payer, plan.price);
            return;
        }
        self.execute_buy(&recipient, &payer, &token_id, &plan);
    }

    /// Pull ledger: pay the predecessor. One yocto guards against a leaked
    /// function-call key draining a balance.
    #[payable]
    pub fn withdraw(&mut self) {
        assert_one_yocto();
        let account = env::predecessor_account_id();
        self.withdraw_credit(&account);
    }

    /// Permissionless: pay a named account. It grants no new authority because
    /// the destination is fixed by the call.
    pub fn withdraw_for(&mut self, account: AccountId) {
        self.withdraw_credit(&account);
    }

    /// Permissionless: pay the pinned treasury in full. Full sweep so nobody
    /// can grief with dust.
    pub fn sweep_treasury(&mut self) {
        let amount = self.treasury_balance;
        require!(amount.0 != 0, "NothingToWithdraw");
        self.treasury_balance = U128(0);
        let treasury = self.treasury.clone();
        SteleEvent::TreasurySwept { treasury: treasury.clone(), amount }.emit();
        self.pay_out(&treasury, amount);
    }

    /// Owner-only treasury withdrawal. On a native site any attached value is
    /// booked to the treasury balance, mirroring the origin `receive()`; on an
    /// FT site native value is refused because the ledger is token-denominated.
    #[payable]
    pub fn withdraw_treasury(&mut self, amount: U128) {
        Self::require_owner();
        require!(amount.0 != 0, "NothingToWithdraw");
        let remaining = self
            .treasury_balance
            .0
            .checked_sub(amount.0)
            .unwrap_or_else(|| env::panic_str("InsufficientTreasuryBalance"));
        let attached = env::attached_deposit();
        self.treasury_balance = if self.settlement_ft.is_some() {
            require!(attached.is_zero(), "NativeNotAccepted");
            U128(remaining)
        } else {
            U128(
                remaining
                    .checked_add(attached.as_yoctonear())
                    .unwrap_or_else(|| env::panic_str("LedgerOverflow")),
            )
        };
        let treasury = self.treasury.clone();
        SteleEvent::TreasurySwept { treasury: treasury.clone(), amount }.emit();
        self.pay_out(&treasury, amount);
    }

    /// Replaces the whole configuration. The ratchet block runs before the
    /// absolute clamps (origin error order), and the floor-policy ratchet runs
    /// too because this setter also carries the floor knobs.
    pub fn set_economics(&mut self, next: SiteConfig) {
        Self::require_owner();
        terms::check_economics(&self.config, &next, self.terms_locked);
        if self.terms_locked {
            terms::check_floor_policy(&self.config, &next);
        }
        terms::check_economics_absolute(&next, self.protocol_bps, self.protocol_rent_bps);

        let rent_changed = self.config.site_rent_bps != next.site_rent_bps
            || self.config.min_rent_bps != next.min_rent_bps
            || self.config.max_rental_term_secs != next.max_rental_term_secs;
        self.config = next;

        SteleEvent::EconomicsChanged {
            take_bps: self.config.take_bps,
            payout_bps: self.config.payout_bps,
            reversion_bps: self.config.reversion_bps,
            max_reversion_weeks: self.config.max_reversion_weeks,
        }
        .emit();
        if rent_changed {
            SteleEvent::RentalTermsChanged {
                site_rent_bps: self.config.site_rent_bps,
                max_rental_term_secs: self.config.max_rental_term_secs,
                min_rent_bps: self.config.min_rent_bps,
            }
            .emit();
        }
    }

    /// Origin `setRentalTerms`: freely mutable in both directions even after
    /// the lock, because a rent-term change cannot touch an active tenancy (the
    /// fee is charged at rent time and snapshotted per listing).
    pub fn set_rental_terms(
        &mut self,
        site_rent_bps: u64,
        max_rental_term_secs: u64,
        min_rent_bps: u64,
    ) {
        Self::require_owner();
        let next = SiteConfig {
            site_rent_bps,
            max_rental_term_secs,
            min_rent_bps,
            ..self.config.clone()
        };
        terms::check_rent_terms(&next, self.protocol_rent_bps);
        self.config = next;
        SteleEvent::RentalTermsChanged {
            site_rent_bps: self.config.site_rent_bps,
            max_rental_term_secs: self.config.max_rental_term_secs,
            min_rent_bps: self.config.min_rent_bps,
        }
        .emit();
    }

    /// Origin `setFloorPolicy`: the floor lever and ask cap. After the lock
    /// they may only tighten; before it any configuration inside the absolute
    /// band is legal (passing `prev == next` degrades the ratchet to the
    /// absolute check).
    pub fn set_floor_policy(
        &mut self,
        floor_delta_bps: u64,
        floor_change_cooldown_secs: u64,
        max_ask_bps: u64,
    ) {
        Self::require_owner();
        let next = SiteConfig {
            floor_delta_bps,
            floor_change_cooldown_secs,
            max_ask_bps,
            ..self.config.clone()
        };
        if self.terms_locked {
            terms::check_floor_policy(&self.config, &next);
        } else {
            terms::check_floor_policy(&next, &next);
        }
        self.config = next;
    }


    /// Origin `setTreasury`; blocked forever once a factory pinned the vault.
    pub fn set_treasury(&mut self, treasury: AccountId) {
        Self::require_owner();
        require!(!self.treasury_pinned, "TreasuryPinned");
        self.treasury = treasury.clone();
        SteleEvent::TreasurySet { treasury }.emit();
    }

    pub fn set_paused(&mut self, paused: bool) {
        Self::require_owner();
        self.paused = paused;
        SteleEvent::PausedSet { paused }.emit();
    }

    /// Two-argument form: a new default floor must clear `min_slot_floor`
    /// whenever registration is being opened.
    pub fn set_open_registration(&mut self, open: bool, default_floor: U128) {
        Self::require_owner();
        if open {
            require!(default_floor.0 >= self.min_slot_floor.0, "InvalidFloor");
        }
        self.open_registration = open;
        self.default_floor = default_floor;
        SteleEvent::OpenRegistrationSet {
            open,
            default_floor,
        }
        .emit();
    }

    /// `base_uri` lives in the NEP-177 contract metadata, not in state.
    pub fn set_base_uri(&mut self, uri: String) {
        Self::require_owner();
        let mut metadata = self.contract_metadata();
        metadata.base_uri = if uri.is_empty() { None } else { Some(uri.clone()) };
        self.set_contract_metadata(&metadata);
        SteleEvent::BaseTokenUriSet { uri }.emit();
    }

    /// Display-only royalty analog: `royalty_info` is never deducted.
    pub fn set_royalty(&mut self, bps: u16) {
        Self::require_owner();
        require!(bps <= MAX_ROYALTY_BPS, "InvalidConfig");
        self.treasury_royalty_bps = bps;
        SteleEvent::RoyaltySet { bps }.emit();
    }

    // Tenancy

    /// Origin `listForRent`: the owner publishes a daily rate and a duration
    /// cap. `rate_per_day == 0` delists. One yocto so a leaked function-call
    /// key cannot reprice a listing.
    #[payable]
    pub fn list_for_rent(
        &mut self,
        token_id: TokenId,
        rate_per_day: U128,
        max_duration_secs: u64,
    ) {
        let owner = self
            .token_owner(&token_id)
            .unwrap_or_else(|| env::panic_str("NotSlotOwner"));
        require!(owner == env::predecessor_account_id(), "NotSlotOwner");
        assert_one_yocto();
        let slot = self
            .slots
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));
        require!(slot.registered, "SlotNotRegistered");

        if rate_per_day.0 == 0 {
            self.listings.remove(&token_id);
            SteleEvent::RentalListed {
                token_id,
                owner,
                rate_per_day: U128(0),
                max_duration_secs: 0,
            }
            .emit();
            return;
        }

        // `site_rent_bps` is freely mutable, so the band is re-checked here: the
        // fee snapshotted below is what the tenant will actually be charged.
        terms::check_rent_terms(&self.config, self.protocol_rent_bps);
        require!(
            rentals::duration_ok(max_duration_secs, self.config.max_rental_term_secs),
            "TermTooLong"
        );
        let effective_floor = self.take_quote(&slot).effective_floor;
        require!(
            rentals::hourly_floor(
                U256::from(rate_per_day.0),
                self.config.min_rent_bps,
                effective_floor,
            ),
            "RateBelowFloor"
        );

        // A new owner inheriting a live tenancy must be able to re-list so the
        // tenant can extend (origin has no live-term guard here). The tenant is
        // protected by `expected_rate_per_day` and the per-listing fee snapshot,
        // which is taken below and never rewritten under a live term.

        let fee_bps = u16::try_from(self.config.site_rent_bps)
            .unwrap_or_else(|_| env::panic_str("InvalidConfig"));
        self.listings.insert(
            token_id.clone(),
            Listing {
                rate_per_day,
                max_duration_secs,
                fee_bps,
            },
        );
        SteleEvent::RentalListed {
            token_id,
            owner,
            rate_per_day,
            max_duration_secs,
        }
        .emit();
    }

    /// Origin `rent`, native branch: opens a term against the owner's listing.
    /// Anyone may rent; the attached deposit is a budget (the change is
    /// credited back) and the gross cost splits into protocol, site and the
    /// escrowed net.
    /// An existing rental entry blocks a new term even after it lapses —
    /// permissionless `end_rental` is what returns the slot to the market.
    /// The FT branch is `ft_on_transfer` calling [`Contract::rent_inner`].
    #[payable]
    pub fn rent(&mut self, token_id: TokenId, duration_secs: u64, expected_rate_per_day: U128) {
        require!(self.settlement_ft.is_none(), "NativeNotAccepted");
        let payer = env::predecessor_account_id();
        let refund = self.rent_inner(
            &payer,
            &payer,
            token_id,
            duration_secs,
            expected_rate_per_day,
            env::attached_deposit().as_yoctonear(),
        );
        // The shared path books native change to `payer` and returns zero; this
        // guard keeps the wrapper's promise local so an overpayment can never be
        // dropped here even if that routing changes.
        if refund.0 != 0 {
            self.credit(&payer, refund);
        }
    }

    /// Origin `extendRental`, native branch: the tenant buys the NEXT window at
    /// the listing's current rate. The cap binds the remaining window (the term
    /// restarts from now), so repeated extensions can never push a buyer past
    /// the listing's advertised term.
    /// Settle-then-restart: the outgoing term is flushed first, then its
    /// unearned remainder rolls forward as the carry of a fresh linear term.
    /// The FT branch is `ft_on_transfer` calling [`Contract::extend_inner`].
    #[payable]
    pub fn extend_rental(
        &mut self,
        token_id: TokenId,
        duration_secs: u64,
        expected_rate_per_day: U128,
    ) {
        require!(self.settlement_ft.is_none(), "NativeNotAccepted");
        let payer = env::predecessor_account_id();
        let refund = self.extend_inner(
            &payer,
            &payer,
            token_id,
            duration_secs,
            expected_rate_per_day,
            env::attached_deposit().as_yoctonear(),
        );
        // Same contract as `rent`: the wrapper never drops a refund.
        if refund.0 != 0 {
            self.credit(&payer, refund);
        }
    }

    /// Origin `claimRent`: permissionless, and always credits the CURRENT owner
    /// rather than the caller, so a keeper can sweep a whole directory without
    /// knowing who owns what.
    pub fn claim_rent(&mut self, token_id: TokenId) {
        let amount = self.settle_rent(&token_id, env::block_timestamp());
        require!(amount.0 != 0, "NothingToWithdraw");
        let owner = self
            .token_owner(&token_id)
            .unwrap_or_else(|| env::panic_str("NotSlotOwner"));
        SteleEvent::RentClaimed {
            token_id,
            owner,
            amount,
        }
        .emit();
    }

    /// Origin `endRental`: permissionless once the term lapses. The unclaimed
    /// remainder is settled to the owner FIRST (the term is about to be
    /// dropped), then the owner's content hash is restored and the tenancy
    /// cleared so the slot can be rented again.
    pub fn end_rental(&mut self, token_id: TokenId) {
        let now_ns = env::block_timestamp();
        let rental = self
            .rentals
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("NotForRent"));
        require!(now_ns >= rental.expiry, "RentalActive");
        self.settle_rent(&token_id, now_ns);

        let mut slot = self
            .slots
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));
        slot.page_hash = rental.owner_hash_snapshot.clone();
        // Bumped so the restore flows through the same content gate as an edit:
        // the render path must not have a second way in.
        slot.content_version = slot
            .content_version
            .checked_add(1)
            .unwrap_or_else(|| env::panic_str("ContentVersionOverflow"));
        self.slots.insert(token_id.clone(), slot);
        self.rentals.remove(&token_id);
        self.sync_token_expiry(&token_id, None);

        SteleEvent::RentalEnded {
            token_id: token_id.clone(),
            restored_hash: rental.owner_hash_snapshot,
        }
        .emit();
        SteleEvent::TenancyChanged {
            token_id,
            tenant: None,
            expiry_ns: U64::from(0u64),
        }
        .emit();
    }

    /// Origin `edit`: the FCK-gated content write. NON-payable on purpose — a
    /// function-call key cannot attach a deposit, so a leaked session key can
    /// never move money, and the contract-side tenancy check is authoritative.
    pub fn edit(&mut self, token_id: TokenId, page_hash: String) {
        let editor = env::predecessor_account_id();
        if !self.can_edit(token_id.clone(), editor.clone()) {
            if self.token_owner(&token_id).is_none() {
                env::panic_str("NotSlotOwner");
            }
            env::panic_str("NotSlotEditor");
        }
        let mut slot = self
            .slots
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));
        slot.page_hash = page_hash.clone();
        slot.content_version = slot
            .content_version
            .checked_add(1)
            .unwrap_or_else(|| env::panic_str("ContentVersionOverflow"));
        let content_version = slot.content_version;
        self.slots.insert(token_id.clone(), slot);
        SteleEvent::SlotEdited {
            token_id,
            editor,
            page_hash,
            content_version,
            timestamp_ns: env::block_timestamp().into(),
        }
        .emit();
    }

    /// Origin `canEdit`: during a live term the TENANT edits and the owner is
    /// locked out. Otherwise the owner, or a grant that the current owner made
    /// and that no take has invalidated.
    pub fn can_edit(&self, token_id: TokenId, account: AccountId) -> bool {
        let Some(owner) = self.token_owner(&token_id) else {
            return false;
        };
        if let Some(rental) = self.rentals.get(&token_id) {
            if env::block_timestamp() < rental.expiry {
                return account == rental.tenant;
            }
        }
        if account == owner {
            return true;
        }
        let Some(slot) = self.slots.get(&token_id) else {
            return false;
        };
        match self.editor_grants.get(&token_id) {
            Some(grant) => {
                grant.editor == account
                    && grant.grantor == owner
                    && grant.granted_at_takes == slot.takes
            }
            None => false,
        }
    }

    /// Origin `setEditor`: one yocto, owner only. The grant records the take
    /// counter, so the next sale kills it without a cleanup call.
    #[payable]
    pub fn set_editor(&mut self, token_id: TokenId, editor: AccountId) {
        let owner = self
            .token_owner(&token_id)
            .unwrap_or_else(|| env::panic_str("NotSlotOwner"));
        require!(owner == env::predecessor_account_id(), "NotSlotOwner");
        assert_one_yocto();
        let slot = self
            .slots
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));
        let granted_at_takes = slot.takes;
        self.editor_grants.insert(
            token_id.clone(),
            EditorGrant {
                editor: editor.clone(),
                grantor: owner.clone(),
                granted_at_takes,
            },
        );
        SteleEvent::EditorSet {
            token_id,
            editor,
            grantor: owner,
            granted_at_takes,
        }
        .emit();
    }

    /// Origin `editorOf`: the granted editor, but only while the grant is live.
    pub fn editor_of(&self, token_id: TokenId) -> Option<AccountId> {
        let grant = self.editor_grants.get(&token_id)?;
        if self.can_edit(token_id, grant.editor.clone()) {
            Some(grant.editor.clone())
        } else {
            None
        }
    }
}

/// Internals: validation, pricing at the boundary, booking and payout. These
/// are not exported (`#[near]` is deliberately absent).
impl Contract {
    /// Registers a batch, used by both `new` and the guarded external method.
    fn register_slots_inner(&mut self, keys: Vec<TokenId>, floors: Vec<U128>) {
        require!(keys.len() == floors.len(), "LengthMismatch");
        for (key, floor) in keys.into_iter().zip(floors) {
            self.register_one(key, floor);
        }
    }

    /// Origin `_register`, one key at a time. Duplicates are rejected whether
    /// they appear earlier in the batch or in an earlier call.
    fn register_one(&mut self, key: TokenId, floor: U128) {
        require!(floor.0 >= self.min_slot_floor.0, "InvalidFloor");
        require!(
            !self.slots.get(&key).map(|slot| slot.registered).unwrap_or(false),
            "SlotAlreadyRegistered"
        );
        let now_ns = env::block_timestamp();
        self.slots.insert(
            key.clone(),
            Slot {
                last_price: floor,
                base_price: floor,
                page_hash: String::new(),
                ask_floor: floor,
                last_purchase_at_ns: now_ns,
                floor_updated_at_ns: now_ns,
                content_version: 0,
                takes: 0,
                registered: true,
                available: true,
            },
        );
        self.registered.push(key.clone());
        SteleEvent::SlotRegistered {
            token_id: key,
            floor,
            timestamp_ns: now_ns.into(),
        }
        .emit();
    }

    /// All the buy guards plus the settled split, computed once so the money
    /// legs cannot drift between the planning receipt and the callback.
    fn plan_buy(
        &mut self,
        token_id: &TokenId,
        max_price: U128,
        expected_terms: &str,
        deadline_ns: u64,
    ) -> BuyPlan {
        require!(env::block_timestamp() <= deadline_ns, "DeadlineExpired");
        require!(!self.paused, "Paused");
        require!(
            self.encumbrance_hash(token_id.clone()) == expected_terms,
            "TermsChanged"
        );

        let registered = self
            .slots
            .get(token_id)
            .map(|slot| slot.registered)
            .unwrap_or(false);
        if !registered {
            require!(self.open_registration, "SlotNotRegistered");
            let default_floor = self.default_floor;
            self.register_one(token_id.clone(), default_floor);
        }
        let slot = self
            .slots
            .get(token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));
        let previous_owner = self.token_owner(token_id);
        let is_unclaimed = previous_owner.is_none();

        if is_unclaimed {
            require!(slot.available, "SlotUnavailable");
        } else {
            let now_secs = env::block_timestamp() / NS_PER_SEC;
            let last_secs = slot.last_purchase_at_ns / NS_PER_SEC;
            require!(
                now_secs >= last_secs.saturating_add(self.config.cooldown_secs),
                "CooldownActive"
            );
        }

        let take = self.take_quote(&slot);
        let split = pricing::compute_split(
            take.effective_floor,
            take.price,
            is_unclaimed,
            self.config.payout_bps,
            self.protocol_bps,
        );
        let charged = pricing::to_u128(split.charged, "charged price");
        require!(charged <= max_price.0, "SlippageExceeded");

        BuyPlan {
            price: U128(charged),
            payout: U128(pricing::to_u128(split.payout, "payout")),
            protocol_cut: U128(pricing::to_u128(split.protocol_cut, "protocol cut")),
            site_cut: U128(pricing::to_u128(split.site_cut, "site cut")),
            previous_owner,
        }
    }

    /// The single money spine. `charged` is the native attached deposit or the
    /// transferred FT amount; it is a budget, never the price.
    /// Returns `Value(price)` for an inline native settlement,
    /// `Value(change)` for an FT settlement (the NEP-141 refund convention) and
    /// `Promise` when the recipient's storage top-up needs a second receipt.
    pub(crate) fn settle_buy(
        &mut self,
        recipient: AccountId,
        token_id: TokenId,
        max_price: U128,
        expected_terms: String,
        deadline_ns: u64,
        charged: U128,
        is_ft: bool,
    ) -> PromiseOrValue<U128> {
        if is_ft {
            require!(self.settlement_ft.is_some(), "FtNotAccepted");
            let balance = self
                .get_storage_balance(recipient.as_ref())
                .unwrap_or_else(|_| env::panic_str("RecipientNotRegistered"));
            require!(
                balance.available >= self.get_storage_balance_bounds().min,
                "RecipientNotRegistered"
            );
        } else {
            require!(self.settlement_ft.is_none(), "NativeNotAccepted");
        }

        let plan = self.plan_buy(&token_id, max_price, &expected_terms, deadline_ns);
        let price = plan.price.0;
        let budget = charged.0;
        // The FT path's predecessor is the settlement token contract, so the
        // account that actually funded the buy is the sender/recipient.
        let payer = if is_ft {
            recipient.clone()
        } else {
            env::predecessor_account_id()
        };

        if is_ft {
            if budget < price {
                return PromiseOrValue::Value(U128(budget));
            }
            self.execute_buy(&recipient, &payer, &token_id, &plan);
            return PromiseOrValue::Value(U128(budget - price));
        }

        // A self-take moves no ownership, so it needs no storage headroom;
        // every other recipient must be able to hold one token.
        let top_up = if plan.previous_owner.as_ref() == Some(&recipient) {
            NearToken::from_yoctonear(0)
        } else {
            self.storage_top_up(&recipient)
        };
        let needed = price
            .checked_add(top_up.as_yoctonear())
            .unwrap_or_else(|| env::panic_str("AmountOverflow"));
        require!(budget >= needed, "InsufficientPayment");
        let change = budget - needed;
        if change > 0 {
            self.credit(&payer, U128(change));
        }

        if top_up.is_zero() {
            self.execute_buy(&recipient, &payer, &token_id, &plan);
            return PromiseOrValue::Value(U128(price));
        }

        // `registration_only` is false on purpose: `bounds.bound(attached, true)`
        // demands the full `min` even when the account is registered and only
        // short of one token's bytes, which would reject a legitimate top-up.
        let deposit_promise = ext_nep145::ext(env::current_account_id())
            .with_static_gas(Gas::from_tgas(25))
            .with_attached_deposit(top_up)
            .storage_deposit(Some(recipient.clone()), Some(false));
        let callback = Self::ext(env::current_account_id())
            .with_static_gas(Gas::from_tgas(60))
            .settle_buy_callback(recipient, payer, token_id, plan, U128(top_up.as_yoctonear()));
        PromiseOrValue::Promise(deposit_promise.then(callback))
    }

    /// Books a settled buy: split legs first, then slot bookkeeping, then the
    /// ownership change. Called inline or from `settle_buy_callback`.
    fn execute_buy(
        &mut self,
        recipient: &AccountId,
        payer: &AccountId,
        token_id: &TokenId,
        plan: &BuyPlan,
    ) {
        let now_ns = env::block_timestamp();
        // Doc C48: a sale settles the outgoing owner's rent before the split
        // credits them the payout. The tenancy itself survives the sale; only
        // the listing is cleared below.
        self.settle_rent(token_id, now_ns);

        if let Some(previous_owner) = &plan.previous_owner {
            self.credit(previous_owner, plan.payout);
        }
        if plan.protocol_cut.0 != 0 {
            let protocol_treasury = self.protocol_treasury.clone();
            self.credit(&protocol_treasury, plan.protocol_cut);
        }
        self.treasury_balance = U128(
            self.treasury_balance
                .0
                .checked_add(plan.site_cut.0)
                .unwrap_or_else(|| env::panic_str("LedgerOverflow")),
        );

        let mut slot = self
            .slots
            .get(token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));
        slot.last_price = plan.price;
        slot.last_purchase_at_ns = now_ns;
        slot.takes = slot
            .takes
            .checked_add(1)
            .unwrap_or_else(|| env::panic_str("TakeCounterOverflow"));
        // A new owner inherits no ask (origin clears it with the sale).
        slot.ask_floor = U128(0);
        self.slots.insert(token_id.clone(), slot);

        // A sale clears the listing; an active rental survives with its term,
        // so a tenancy a take could destroy never exists.
        self.listings.remove(token_id);

        match &plan.previous_owner {
            None => {
                let metadata = self.take_metadata(token_id, now_ns);
                self.mint_with_metadata(token_id, recipient.as_ref(), &metadata)
                    .unwrap_or_else(|error| env::panic_str(&error.to_string()));
                if !self.terms_locked {
                    self.terms_locked = true;
                    SteleEvent::TermsLocked.emit();
                }
            }
            Some(previous_owner) => {
                if previous_owner == recipient {
                    // A self-take changes nothing about ownership; it still
                    // clears approvals, as the origin transfer would.
                    self.revoke_all_unchecked(token_id);
                } else {
                    let transfer = Nep171Transfer {
                        authorization: Nep171TransferAuthorization::Owner,
                        sender_id: Cow::Owned(previous_owner.clone()),
                        receiver_id: Cow::Owned(recipient.clone()),
                        token_id: token_id.clone(),
                        memo: None,
                        msg: None,
                        revert: false,
                    };
                    self.external_transfer(&transfer)
                        .unwrap_or_else(|error| env::panic_str(&error.to_string()));
                }
            }
        }

        SteleEvent::SlotBought {
            token_id: token_id.clone(),
            buyer: recipient.clone(),
            previous_owner: plan.previous_owner.clone(),
            payer: payer.clone(),
            price: plan.price,
            payout: plan.payout,
            timestamp_ns: now_ns.into(),
        }
        .emit();
    }

    /// NEP-177 metadata at the boundary: milliseconds, `starts_at` absent, and
    /// `expires_at` mirroring a live tenancy (the tenant keeps the term).
    fn take_metadata(&self, token_id: &TokenId, now_ns: u64) -> TokenMetadata {
        let mut metadata = TokenMetadata::new()
            .title(token_id.clone())
            .description("stele slot")
            .copies(1u64)
            .issued_at(now_ns / NS_PER_MS);
        if let Some(expiry_ns) = self
            .rentals
            .get(token_id)
            .map(|rental| rental.expiry)
            .filter(|expiry| *expiry > now_ns)
        {
            metadata = metadata.expires_at(expiry_ns / NS_PER_MS);
        }
        metadata
    }

    /// Mirrors a live tenancy into NEP-177 `expires_at` (milliseconds) without
    /// touching the rest of the token metadata; `None` clears the field alone.
    /// The unchecked setter is the right one here: the token exists (a tenancy
    /// requires an owner), and clearing a field needs the `Option` form that
    /// `set_token_metadata` does not take.
    fn sync_token_expiry(&mut self, token_id: &TokenId, expires_at_ns: Option<u64>) {
        let Some(mut metadata) = self.token_metadata(token_id) else {
            return;
        };
        metadata.expires_at = expires_at_ns.map(|ns| U64::from(ns / NS_PER_MS));
        self.set_token_metadata_unchecked(token_id, Some(&metadata));
    }

    /// Books what a term has earned into the CURRENT owner's pull ledger and
    /// marks it claimed. Returns the delta moved out of rent escrow; an absent
    /// or cleared term returns zero.
    /// Shared by `claim_rent`, `extend_rental`, `end_rental` and `execute_buy`,
    /// so every path that can change who is owed the stream flushes it first.
    /// Accrual is linear over the term and clamps at expiry, which makes the
    /// lapsed-term settle drain the escrow to exactly zero.
    pub(crate) fn settle_rent(&mut self, token_id: &TokenId, now_ns: u64) -> U128 {
        let Some(mut rental) = self.rentals.get(token_id).cloned() else {
            return U128(0);
        };
        if rental.expiry == 0 {
            return U128(0);
        }
        // Seconds, exactly like the origin and the parity vectors; truncating
        // both term bounds by the same divisor keeps `expiry - start` exact.
        let accrued = rentals::accrued_of(
            U256::from(rental.prepaid.0),
            rental.start / NS_PER_SEC,
            rental.expiry / NS_PER_SEC,
            now_ns / NS_PER_SEC,
        );
        let accrued = pricing::to_u128(accrued, "accrued rent");
        let delta = accrued
            .checked_sub(rental.claimed.0)
            .unwrap_or_else(|| env::panic_str("RentClaimUnderflow"));
        if delta == 0 {
            return U128(0);
        }

        rental.claimed = U128(accrued);
        self.rentals.insert(token_id.clone(), rental);
        self.total_escrowed_rent = U128(
            self.total_escrowed_rent
                .0
                .checked_sub(delta)
                .unwrap_or_else(|| env::panic_str("RentEscrowUnderflow")),
        );
        let owner = self
            .token_owner(token_id)
            .unwrap_or_else(|| env::panic_str("NotSlotOwner"));
        self.credit(&owner, U128(delta));
        U128(delta)
    }

    /// Routes an overpayment. On a native site the money is already inside
    /// this contract and a value return cannot pay NEAR, so it becomes a
    /// pull-ledger credit; on an FT site the amount goes back to
    /// `ft_on_transfer`, which returns it to the sender. The branches are
    /// disjoint by construction: `rent`/`extend_rental` require a native site
    /// and `ft_on_transfer` refuses one.
    fn refund_or_credit(&mut self, payer: &AccountId, refund: u128) -> U128 {
        if self.settlement_ft.is_none() {
            if refund > 0 {
                self.credit(payer, U128(refund));
            }
            U128(0)
        } else {
            U128(refund)
        }
    }

    /// Shared `rent` path: `paid` is the native attached deposit or the FT
    /// amount, `tenant` the renter, `payer` the account the change is owed to.
    /// Returns what `ft_on_transfer` must hand back (zero on the native path,
    /// where [`Contract::refund_or_credit`] books it instead).
    pub(crate) fn rent_inner(
        &mut self,
        payer: &AccountId,
        tenant: &AccountId,
        token_id: TokenId,
        duration_secs: u64,
        expected_rate_per_day: U128,
        paid: u128,
    ) -> U128 {
        require!(!self.paused, "Paused");
        // Cannot rent an unclaimed position: there is nobody to pay.
        require!(self.token_owner(&token_id).is_some(), "NotSlotOwner");
        require!(self.rentals.get(&token_id).is_none(), "RentalActive");

        let listing = self
            .listings
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("NotForRent"));
        require!(listing.rate_per_day.0 != 0, "NotForRent");
        require!(listing.rate_per_day == expected_rate_per_day, "RateChanged");
        require!(
            rentals::duration_ok(duration_secs, listing.max_duration_secs),
            "TermTooLong"
        );

        let cost = pricing::to_u128(
            rentals::rent_cost(U256::from(listing.rate_per_day.0), duration_secs),
            "rent cost",
        );
        require!(paid >= cost, "InsufficientPayment");
        let refund = paid - cost;

        let split = rentals::rent_split(
            U256::from(cost),
            self.protocol_rent_bps,
            u64::from(listing.fee_bps),
        );
        let protocol_cut = pricing::to_u128(split.protocol_cut, "protocol rent cut");
        let site_cut = pricing::to_u128(split.site_cut, "site rent cut");
        let net = pricing::to_u128(split.net, "escrowed rent");
        if protocol_cut != 0 {
            let protocol_treasury = self.protocol_treasury.clone();
            self.credit(&protocol_treasury, U128(protocol_cut));
        }
        self.treasury_balance = U128(
            self.treasury_balance
                .0
                .checked_add(site_cut)
                .unwrap_or_else(|| env::panic_str("LedgerOverflow")),
        );
        self.total_escrowed_rent = U128(
            self.total_escrowed_rent
                .0
                .checked_add(net)
                .unwrap_or_else(|| env::panic_str("RentEscrowOverflow")),
        );

        let now_ns = env::block_timestamp();
        let duration_ns = duration_secs
            .checked_mul(NS_PER_SEC)
            .unwrap_or_else(|| env::panic_str("ExpiryOverflow"));
        let expiry_ns = now_ns
            .checked_add(duration_ns)
            .unwrap_or_else(|| env::panic_str("ExpiryOverflow"));
        let slot = self
            .slots
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("SlotNotRegistered"));
        self.rentals.insert(
            token_id.clone(),
            Rental {
                tenant: tenant.clone(),
                start: now_ns,
                expiry: expiry_ns,
                prepaid: U128(net),
                claimed: U128(0),
                owner_hash_snapshot: slot.page_hash,
            },
        );
        self.sync_token_expiry(&token_id, Some(expiry_ns));

        SteleEvent::SlotRented {
            token_id: token_id.clone(),
            tenant: tenant.clone(),
            expiry_ns: U64::from(expiry_ns),
            cost: U128(cost),
        }
        .emit();
        SteleEvent::TenancyChanged {
            token_id,
            tenant: Some(tenant.clone()),
            expiry_ns: U64::from(expiry_ns),
        }
        .emit();

        self.refund_or_credit(payer, refund)
    }

    /// Shared `extend_rental` path. Settle-then-restart: the outgoing term is
    /// flushed first, then its unearned remainder rolls forward as the carry of
    /// a fresh linear term. The cap binds the remaining window.
    pub(crate) fn extend_inner(
        &mut self,
        payer: &AccountId,
        tenant: &AccountId,
        token_id: TokenId,
        duration_secs: u64,
        expected_rate_per_day: U128,
        paid: u128,
    ) -> U128 {
        require!(!self.paused, "Paused");
        let now_ns = env::block_timestamp();
        let rental = self
            .rentals
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("RentalExpired"));
        require!(now_ns < rental.expiry, "RentalExpired");
        require!(&rental.tenant == tenant, "NotSlotTenant");

        let listing = self
            .listings
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("NotForRent"));
        require!(listing.rate_per_day.0 != 0, "NotForRent");
        require!(listing.rate_per_day == expected_rate_per_day, "RateChanged");
        require!(
            rentals::duration_ok(duration_secs, self.config.max_rental_term_secs),
            "TermTooLong"
        );

        // Flush what the outgoing term earned before its numbers move; the
        // carry is what it never earned, so it must not be refunded.
        self.settle_rent(&token_id, now_ns);
        let rental = self
            .rentals
            .get(&token_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("RentalExpired"));
        let carry = pricing::to_u128(
            rentals::extend_carry(
                U256::from(rental.prepaid.0),
                U256::from(rental.claimed.0),
                rental.start / NS_PER_SEC,
                rental.expiry / NS_PER_SEC,
                now_ns / NS_PER_SEC,
            ),
            "rent carry",
        );

        let cost = pricing::to_u128(
            rentals::rent_cost(U256::from(listing.rate_per_day.0), duration_secs),
            "rent cost",
        );
        require!(paid >= cost, "InsufficientPayment");
        let refund = paid - cost;

        let split = rentals::rent_split(
            U256::from(cost),
            self.protocol_rent_bps,
            u64::from(listing.fee_bps),
        );
        let protocol_cut = pricing::to_u128(split.protocol_cut, "protocol rent cut");
        let site_cut = pricing::to_u128(split.site_cut, "site rent cut");
        let net = pricing::to_u128(split.net, "escrowed rent");
        if protocol_cut != 0 {
            let protocol_treasury = self.protocol_treasury.clone();
            self.credit(&protocol_treasury, U128(protocol_cut));
        }
        self.treasury_balance = U128(
            self.treasury_balance
                .0
                .checked_add(site_cut)
                .unwrap_or_else(|| env::panic_str("LedgerOverflow")),
        );
        self.total_escrowed_rent = U128(
            self.total_escrowed_rent
                .0
                .checked_add(net)
                .unwrap_or_else(|| env::panic_str("RentEscrowOverflow")),
        );

        let prepaid = carry
            .checked_add(net)
            .unwrap_or_else(|| env::panic_str("RentEscrowOverflow"));
        let duration_ns = duration_secs
            .checked_mul(NS_PER_SEC)
            .unwrap_or_else(|| env::panic_str("ExpiryOverflow"));
        let expiry_ns = now_ns
            .checked_add(duration_ns)
            .unwrap_or_else(|| env::panic_str("ExpiryOverflow"));
        self.rentals.insert(
            token_id.clone(),
            Rental {
                tenant: tenant.clone(),
                start: now_ns,
                expiry: expiry_ns,
                prepaid: U128(prepaid),
                claimed: U128(0),
                owner_hash_snapshot: rental.owner_hash_snapshot,
            },
        );
        self.sync_token_expiry(&token_id, Some(expiry_ns));

        SteleEvent::RentalExtended {
            token_id: token_id.clone(),
            tenant: tenant.clone(),
            expiry_ns: U64::from(expiry_ns),
            cost: U128(cost),
        }
        .emit();
        SteleEvent::TenancyChanged {
            token_id,
            tenant: Some(tenant.clone()),
            expiry_ns: U64::from(expiry_ns),
        }
        .emit();

        self.refund_or_credit(payer, refund)
    }

    /// `max(bounds.min - total, token_bytes - available)`, where one token's
    /// bytes are the tutorial's 700-byte bound (7 mNEAR).
    fn storage_shortfall(&self, recipient: &AccountId) -> NearToken {
        let bounds = self.get_storage_balance_bounds();
        let balance = self.get_storage_balance(recipient.as_ref()).ok();
        let zero = NearToken::from_yoctonear(0);
        let total = balance.as_ref().map(|value| value.total).unwrap_or(zero);
        let available = balance
            .as_ref()
            .map(|value| value.available)
            .unwrap_or(zero);
        let token_bytes = NearToken::from_millinear(TOKEN_STORAGE_MILLINEAR);
        bounds
            .min
            .saturating_sub(total)
            .max(token_bytes.saturating_sub(available))
    }

    /// The shortfall actually forwarded to `storage_deposit`: capped by the
    /// bounds maximum, and refused outright when even a full top-up cannot
    /// leave room for one token.
    fn storage_top_up(&self, recipient: &AccountId) -> NearToken {
        let shortfall = self.storage_shortfall(recipient);
        if shortfall.is_zero() {
            return shortfall;
        }
        let balance = self.get_storage_balance(recipient.as_ref()).ok();
        let zero = NearToken::from_yoctonear(0);
        let total = balance.as_ref().map(|value| value.total).unwrap_or(zero);
        let available = balance
            .as_ref()
            .map(|value| value.available)
            .unwrap_or(zero);
        let bounds = self.get_storage_balance_bounds();
        let headroom = bounds
            .max
            .map(|max| max.saturating_sub(total))
            .unwrap_or(NearToken::from_yoctonear(u128::MAX));
        let top_up = shortfall.min(headroom);
        let token_bytes = NearToken::from_millinear(TOKEN_STORAGE_MILLINEAR);
        require!(
            available.saturating_add(top_up) >= token_bytes,
            "RecipientStorageExhausted"
        );
        top_up
    }

    /// Credits the pull ledger. The entry is added before any transfer is
    /// created, so no send can race a second claim.
    fn credit(&mut self, account: &AccountId, amount: U128) {
        if amount.0 == 0 {
            return;
        }
        let current = self
            .pending_withdrawals
            .get(account)
            .map(|value| value.0)
            .unwrap_or(0);
        let updated = current
            .checked_add(amount.0)
            .unwrap_or_else(|| env::panic_str("LedgerOverflow"));
        self.pending_withdrawals.insert(account.clone(), U128(updated));
    }

    /// Deducts before sending. The entry is removed in the same receipt that
    /// creates the transfer, so a failed receipt can never be withdrawn twice.
    fn withdraw_credit(&mut self, account: &AccountId) {
        let amount = self.pending_withdrawals.remove(account).unwrap_or(U128(0));
        require!(amount.0 != 0, "NothingToWithdraw");
        SteleEvent::PayoutWithdrawn { account: account.clone(), amount }.emit();
        self.pay_out(account, amount);
    }

    /// Native `Promise::transfer`, or a NEP-141 `ft_transfer` with the
    /// one-yocto guard the standard requires.
    fn pay_out(&self, account: &AccountId, amount: U128) {
        match &self.settlement_ft {
            Some(settlement_ft) => {
                near_sdk_contract_tools::ft::ext_nep141::ext(settlement_ft.clone())
                    .with_static_gas(Gas::from_tgas(30))
                    .with_attached_deposit(NearToken::from_yoctonear(1))
                    .ft_transfer(account.clone(), amount, None)
                    .detach();
            }
            None => {
                Promise::new(account.clone())
                    .transfer(NearToken::from_yoctonear(amount.0))
                    .detach();
            }
        }
    }

    /// The one place the boundary meets `pricing::compute_take_price`.
    pub(crate) fn take_quote(&self, slot: &Slot) -> TakeQuote {
        let now_secs = env::block_timestamp() / NS_PER_SEC;
        let last_secs = slot.last_purchase_at_ns / NS_PER_SEC;
        let decay_base = pricing::resolve_reversion_base(
            U256::from(slot.ask_floor.0),
            U256::from(slot.last_price.0),
        );
        pricing::compute_take_price(
            decay_base,
            U256::from(slot.base_price.0),
            pricing::compute_elapsed_weeks(now_secs, last_secs),
            self.config.reversion_bps,
            self.config.take_bps,
            u64::from(self.config.max_reversion_weeks),
        )
    }
}
