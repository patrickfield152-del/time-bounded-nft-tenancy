//! stele factory: per-site sub-account deployment, escrow-vault wiring, and
//! the rotatable attestor registry.
//!
//! Port of the origin factory, escrow and attestor contracts. There is
//! no `delegatecall` and no clone-proxy on NEAR, so a site is a direct
//! sub-account of this contract carrying its own copy of the site wasm:
//! `create_account -> transfer -> deploy_contract -> function_call("new")` in
//! one receipt batch, exactly the verified factory pattern. Global contracts
//! are never used (the global NFT primitives are codeless on chain today, and
//! by-hash references cannot be verified on public RPC).
//!
//! ## Code storage
//!
//! The wasm is uploaded once by the owner through `store_site_code` /
//! `store_escrow_code` instead of being baked in with `include_bytes!`. The
//! economics are identical: the bytes are staked on THIS account, so
//! `get_required` quotes `site_code.len() * 1e19` yocto per site (every site
//! gets its own copy, so every site pays for its own code) plus a state
//! allowance. A caller may instead attach more; see `create_site` for where
//! the excess goes.
//!
//! ## Promise discipline
//!
//! Every multi-receipt path ends in a `#[private]` callback that inspects
//! `is_promise_success()` (or a `#[callback_result]`) and refunds the caller
//! when the chain failed. Nothing is credited on a failure path; the only
//! irreversible spend is an account that already exists.

use near_sdk::json_types::{Base64VecU8, U128};
use near_sdk::store::LookupMap;
use near_sdk::utils::is_promise_success;
use near_sdk::{
    env, near, require, serde_json::json, AccountId, BorshStorageKey, Gas, NearToken,
    PanicOnDefault, Promise, PromiseError,
};
use near_sdk::borsh::BorshSerialize;
use near_sdk_contract_tools::owner::Owner as OwnerTrait;
use near_sdk_contract_tools::Owner;

/// The site's init config, duplicated here as wire shape only: a contract that
/// depends on another contract crate inherits its ABI symbols and the
/// `near-sdk/__abi-generate` link step then sees duplicate `__near_abi_*`
/// definitions. The JSON field set is identical to `stele_site::SiteConfig`.
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

/// The vault's bind policy, duplicated for the same ABI reason; wire-identical
/// to `stele_escrow::EscrowParams`.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscrowParams {
    pub window_secs: u64,
    pub claim_delay_secs: u64,
    pub min_transition_secs: u64,
    pub min_booking: U128,
}

/// Storage price per byte: 1e19 yocto (1 NEAR = 10^24 yocto, so 10^5 bytes
/// per NEAR). Code storage is the dominant cost of a factory deployment.
pub const STORAGE_BYTE_COST: u128 = 10_000_000_000_000_000_000;
/// State allowance funded for a new site: 2,000 bytes. A freshly initialized
/// site (config, owner slot, metadata, the registered-slots vector, one slot)
/// fits well inside this, and the site keeps whatever `create_site` does not
/// reserve.
pub const SITE_STATE_ALLOWANCE: u128 = 20_000_000_000_000_000_000_000;
/// State allowance funded for a new escrow vault: 1,000 bytes.
pub const ESCROW_STATE_ALLOWANCE: u128 = 10_000_000_000_000_000_000_000;
/// Kept on the factory per created site for its own bookkeeping (the
/// `vault_of` entry and the callback's storage): 100 bytes.
pub const CREATE_RESERVE: u128 = 1_000_000_000_000_000_000_000;
/// The site's NEP-145 `storage_balance_bounds.max` (21 mNEAR). The factory
/// registers the vault at the site before `bind` so the surrender transfer has
/// a funded receiver; this is the exact amount the site's bound will accept.
pub const SITE_STORAGE_DEPOSIT: u128 = 21_000_000_000_000_000_000_000;
/// Native NEAR is 24 decimals; the factory keys its allow-list with this
/// pseudo-account so one map answers both settlement branches.
pub const NATIVE_TOKEN_KEY: &str = "near";
/// Native NEAR decimals, used to seed the allow-list at `new`.
pub const NATIVE_DECIMALS: u8 = 24;

/// Attestor pointer; `None` means fail-open to the factory owner, as in the
/// origin registry.
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

/// Everything the escrowed-site chain needs to carry between callbacks. It is
/// serialized as JSON callback arguments, so a failed stage can always refund
/// exactly what the caller attached minus what was already spent.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscrowPlan {
    pub vault_id: AccountId,
    pub site_id: AccountId,
    pub payer: AccountId,
    pub owner_id: AccountId,
    pub settlement_token: Option<AccountId>,
    pub min_slot_floor: U128,
    pub config: SiteConfig,
    pub keys: Vec<String>,
    pub floors: Vec<U128>,
    /// The full attached deposit, refunded on a vault-stage failure.
    pub deposit: U128,
    /// Sent to the vault account (code stake + state allowance).
    pub vault_funding: U128,
    /// Sent to the site account (code stake + state allowance - reserve).
    pub site_funding: U128,
}

/// Storage prefixes for the factory-owned maps.
#[derive(BorshSerialize, BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
enum StorageKey {
    AllowedTokens,
    VaultOf,
}

#[near(contract_state)]
#[derive(PanicOnDefault, Owner)]
pub struct Contract {
    /// Admin slot lives in the `Owner` derive; there is no `owner_id` field.
    pub protocol_treasury: AccountId,
    pub protocol_bps: u64,
    pub protocol_rent_bps: u64,
    pub attestor: Option<AccountId>,
    /// Site wasm, uploaded once through `store_site_code`.
    pub site_code: Vec<u8>,
    /// Escrow wasm, uploaded once through `store_escrow_code`.
    pub escrow_code: Vec<u8>,
    /// Settlement asset -> decimals/floor policy. Native NEAR is keyed by
    /// [`NATIVE_TOKEN_KEY`].
    pub allowed_tokens: LookupMap<AccountId, AllowedToken>,
    /// Site account -> escrow vault account.
    pub vault_of: LookupMap<AccountId, AccountId>,
    /// Policy copied into every vault this factory creates.
    pub escrow_params: EscrowParams,
}

#[near]
impl Contract {
    /// The deployer becomes the factory owner. `protocol_bps` and
    /// `protocol_rent_bps` are clamped twice: here, so a bad factory can never
    /// be initialized, and again by every site's `new`.
    #[init]
    pub fn new(
        protocol_treasury: AccountId,
        protocol_bps: u64,
        protocol_rent_bps: u64,
        attestor: Option<AccountId>,
    ) -> Self {
        require!(protocol_bps <= 10_000, "InvalidProtocolBps");
        require!(protocol_rent_bps <= 4_000, "InvalidProtocolRentBps");

        let mut contract = Self {
            protocol_treasury,
            protocol_bps,
            protocol_rent_bps,
            attestor,
            site_code: Vec::new(),
            escrow_code: Vec::new(),
            allowed_tokens: LookupMap::new(StorageKey::AllowedTokens),
            vault_of: LookupMap::new(StorageKey::VaultOf),
            escrow_params: EscrowParams {
                window_secs: 0,
                claim_delay_secs: 0,
                min_transition_secs: 0,
                min_booking: U128(0),
            },
        };
        let owner_id = env::predecessor_account_id();
        OwnerTrait::init(&mut contract, &owner_id);
        contract.allowed_tokens.insert(
            NATIVE_TOKEN_KEY.parse().unwrap_or_else(|_| env::panic_str("BadNativeKey")),
            AllowedToken {
                decimals: NATIVE_DECIMALS,
                min_slot_floor: U128(10u128.pow(20)),
            },
        );
        contract
    }

    /// Owner-only upload of the site wasm. Payable: the attached value stays on
    /// this account and stakes the code bytes.
    #[payable]
    pub fn store_site_code(&mut self, code: Base64VecU8) {
        self.assert_owner();
        require!(!code.0.is_empty(), "EmptyCode");
        self.site_code = code.0;
    }

    /// Owner-only upload of the escrow wasm.
    #[payable]
    pub fn store_escrow_code(&mut self, code: Base64VecU8) {
        self.assert_owner();
        require!(!code.0.is_empty(), "EmptyCode");
        self.escrow_code = code.0;
    }

    /// Owner-only: policy for every vault this factory will create.
    pub fn set_escrow_params(&mut self, params: EscrowParams) {
        self.assert_owner();
        require!(params.window_secs != 0, "InvalidEscrowParams");
        self.escrow_params = params;
    }

    /// Owner-only: allow a settlement asset and derive `10^(decimals - 4)`
    /// from its real precision. A 6-decimal token gets `100`; native is
    /// pre-seeded with 24 => `10^20`. Never `1e14` (the 18-decimal EVM number,
    /// which is 1e-6 of the intended floor on a 6-decimal asset).
    pub fn allow_token(&mut self, token: AccountId, decimals: u8) {
        self.assert_owner();
        require!(decimals >= 4, "InvalidDecimals");
        let exponent = u32::from(decimals - 4);
        let min_slot_floor = 10u128
            .checked_pow(exponent)
            .unwrap_or_else(|| env::panic_str("FloorOverflow"));
        require!(min_slot_floor != 0, "InvalidFloor");
        self.allowed_tokens.insert(
            token,
            AllowedToken {
                decimals,
                min_slot_floor: U128(min_slot_floor),
            },
        );
    }

    /// Owner-only attestor rotation. `None` = nobody, which fails open to the
    /// factory owner on `mark_dark`/`clear_dark`.
    pub fn set_attestor(&mut self, attestor: Option<AccountId>) {
        self.assert_owner();
        self.attestor = attestor;
    }

    pub fn attestor(&self) -> Option<AccountId> {
        self.attestor.clone()
    }

    pub fn vault_of(&self, site_id: AccountId) -> Option<AccountId> {
        self.vault_of.get(&site_id).cloned()
    }

    /// Quote for one non-escrowed site: the per-site code stake plus a state
    /// allowance. `args_len` widens the allowance when the init arguments are
    /// larger than the fixed 2,000-byte assumption, never shrinks it.
    pub fn get_required(&self, args_len: u64) -> NearToken {
        let code_stake = (self.site_code.len() as u128)
            .checked_mul(STORAGE_BYTE_COST)
            .unwrap_or_else(|| env::panic_str("QuoteOverflow"));
        let args_stake = (args_len as u128)
            .checked_mul(STORAGE_BYTE_COST)
            .unwrap_or_else(|| env::panic_str("QuoteOverflow"));
        NearToken::from_yoctonear(code_stake.saturating_add(SITE_STATE_ALLOWANCE.max(args_stake)))
    }

    /// Quote for `create_escrowed_site`: site quote, vault quote, and the
    /// NEP-145 registration the vault needs at the site.
    pub fn get_required_escrowed(&self, args_len: u64) -> NearToken {
        let site = self.get_required(args_len).as_yoctonear();
        let vault_code = (self.escrow_code.len() as u128)
            .checked_mul(STORAGE_BYTE_COST)
            .unwrap_or_else(|| env::panic_str("QuoteOverflow"));
        NearToken::from_yoctonear(
            site.saturating_add(vault_code)
                .saturating_add(ESCROW_STATE_ALLOWANCE)
                .saturating_add(SITE_STORAGE_DEPOSIT),
        )
    }

    /// Deploys a site owned by the caller. `settlement_token = None` selects
    /// the native branch; `Some(token)` must be allow-listed.
    ///
    /// The batch is atomic: if account creation, the code deploy or `new`
    /// fails, every action rolls back, `create_callback` sees the failure and
    /// refunds the whole attached deposit to the payer. Over-attachment beyond
    /// the quote is forwarded to the site (it is the site's balance, and the
    /// callback cannot claw it back).
    #[payable]
    pub fn create_site(
        &mut self,
        site_id: AccountId,
        settlement_token: Option<AccountId>,
        config: SiteConfig,
        keys: Vec<String>,
        floors: Vec<U128>,
    ) -> Promise {
        let caller = env::predecessor_account_id();
        self.create_site_inner(
            caller.clone(),
            caller,
            site_id,
            settlement_token,
            config,
            keys,
            floors,
        )
    }

    /// Deploys a site owned by `owner_id`; the publisher signs nothing and the
    /// payer (the predecessor) gets any failure refund. This is the path a
    /// non-technical product depends on.
    #[payable]
    pub fn create_site_for(
        &mut self,
        owner_id: AccountId,
        site_id: AccountId,
        settlement_token: Option<AccountId>,
        config: SiteConfig,
        keys: Vec<String>,
        floors: Vec<U128>,
    ) -> Promise {
        let payer = env::predecessor_account_id();
        self.create_site_inner(
            owner_id,
            payer,
            site_id,
            settlement_token,
            config,
            keys,
            floors,
        )
    }

    /// Vault -> pinned site -> NEP-145 registration -> bind, one async chain.
    /// The vault is a direct sub-account carrying `escrow_code`; the site's
    /// treasury is forced to the vault and pinned, so the publisher's cut has
    /// exactly one door out of the site.
    #[payable]
    pub fn create_escrowed_site(
        &mut self,
        vault_id: AccountId,
        owner_id: AccountId,
        site_id: AccountId,
        settlement_token: Option<AccountId>,
        config: SiteConfig,
        keys: Vec<String>,
        floors: Vec<U128>,
    ) -> Promise {
        let payer = env::predecessor_account_id();
        self.create_escrowed_site_inner(
            owner_id,
            payer,
            vault_id,
            site_id,
            settlement_token,
            config,
            keys,
            floors,
        )
    }

    /// Attestor gateway to the vault's dark clock. With an attestor set only
    /// that account may call this; with `None` the factory owner may (fail-open
    /// means "nobody" cannot be expressed on chain, so the admin fallback is
    /// the owner, exactly as the origin's zero-address rule intends).
    pub fn mark_dark(&mut self, vault_id: AccountId) -> Promise {
        self.assert_attestor();
        self.forward_dark(vault_id, "mark_dark")
    }

    pub fn clear_dark(&mut self, vault_id: AccountId) -> Promise {
        self.assert_attestor();
        self.forward_dark(vault_id, "clear_dark")
    }

    /// Factory-owner proxy for the vault reserve: the vault only lets its
    /// factory call `withdraw_reserve`, which mirrors the origin reading
    /// `factory.owner()` at call time.
    pub fn withdraw_reserve(&mut self, vault_id: AccountId, amount: U128, to: AccountId) -> Promise {
        self.assert_owner();
        Promise::new(vault_id.clone())
            .function_call(
                "withdraw_reserve".to_owned(),
                json!({ "amount": amount, "to": to }).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(40),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "reserve_callback".to_owned(),
                    json!({ "vault_id": vault_id }).to_string().into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(10),
                ),
            )
    }

    /// Private tail of `create_site`/`create_site_for`: refund the payer when
    /// the creation batch failed. A success has nothing to undo.
    #[private]
    pub fn create_callback(
        &mut self,
        site_id: AccountId,
        owner_id: AccountId,
        payer: AccountId,
        deposit: U128,
    ) {
        if is_promise_success() {
            let _ = (site_id, owner_id);
            return;
        }
        self.refund(&payer, deposit.0);
    }

    /// Stage 1 tail: the vault batch. On failure refund everything; on success
    /// create the pinned site.
    #[private]
    pub fn escrowed_vault_callback(
        &mut self,
        plan: EscrowPlan,
        #[callback_result] created: Result<(), PromiseError>,
    ) {
        if created.is_err() {
            self.refund(&plan.payer, plan.deposit.0);
            return;
        }
        let args = self.site_args(
            &plan.owner_id,
            plan.settlement_token.clone(),
            &self.protocol_treasury,
            Some(plan.vault_id.clone()),
            true,
            &plan.config,
            &plan.keys,
            &plan.floors,
            plan.min_slot_floor,
        );
        Promise::new(plan.site_id.clone())
            .create_account()
            .transfer(NearToken::from_yoctonear(plan.site_funding.0))
            .deploy_contract(self.site_code.clone())
            .function_call(
                "new".to_owned(),
                args,
                NearToken::from_yoctonear(0),
                Gas::from_tgas(60),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "escrowed_site_callback".to_owned(),
                    serde_json::to_vec(&json!({ "plan": plan })).unwrap_or_else(|_| env::panic_str("PlanEncode")),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(40),
                ),
            )
            .detach();
    }

    /// Stage 2 tail: the pinned site batch. On failure the vault exists and is
    /// funded, so only the still-held remainder can be refunded; the orphan
    /// vault is the documented cost of a non-atomic factory on NEAR. On
    /// success register the vault at the site (NEP-145, `registration_only =
    /// false`, the site's `bounds.max` attached) so the surrender transfer has
    /// a funded receiver.
    #[private]
    pub fn escrowed_site_callback(
        &mut self,
        plan: EscrowPlan,
        #[callback_result] created: Result<(), PromiseError>,
    ) {
        if created.is_err() {
            self.refund(
                &plan.payer,
                plan.deposit.0.saturating_sub(plan.vault_funding.0),
            );
            return;
        }
        Promise::new(plan.site_id.clone())
            .function_call(
                "storage_deposit".to_owned(),
                json!({
                    "account_id": plan.vault_id,
                    "registration_only": false,
                })
                .to_string()
                .into_bytes(),
                NearToken::from_yoctonear(SITE_STORAGE_DEPOSIT),
                Gas::from_tgas(50),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "escrowed_registration_callback".to_owned(),
                    serde_json::to_vec(&json!({ "plan": plan })).unwrap_or_else(|_| env::panic_str("PlanEncode")),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(40),
                ),
            )
            .detach();
    }

    /// Stage 3 tail: the vault is registered at the site. Bind it (the vault
    /// verifies the pin in both directions and then books), then record the
    /// site -> vault mapping only if bind succeeded.
    #[private]
    pub fn escrowed_registration_callback(
        &mut self,
        plan: EscrowPlan,
        #[callback_result] registered: Result<(), PromiseError>,
    ) {
        if registered.is_err() {
            self.refund(
                &plan.payer,
                plan.deposit
                    .0
                    .saturating_sub(plan.vault_funding.0)
                    .saturating_sub(plan.site_funding.0),
            );
            return;
        }
        Promise::new(plan.vault_id.clone())
            .function_call(
                "bind".to_owned(),
                json!({
                    "site": plan.site_id,
                    "params": self.escrow_params.clone(),
                })
                .to_string()
                .into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(80),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "escrowed_bind_callback".to_owned(),
                    serde_json::to_vec(&json!({ "plan": plan })).unwrap_or_else(|_| env::panic_str("PlanEncode")),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(40),
                ),
            )
            .detach();
    }

    /// Stage 4 tail: bind succeeded, so the pair is live. Record provenance and
    /// refund whatever the caller attached beyond the spent quote.
    #[private]
    pub fn escrowed_bind_callback(
        &mut self,
        plan: EscrowPlan,
        #[callback_result] bound: Result<(), PromiseError>,
    ) {
        if bound.is_err() {
            self.refund(
                &plan.payer,
                plan.deposit
                    .0
                    .saturating_sub(plan.vault_funding.0)
                    .saturating_sub(plan.site_funding.0),
            );
            return;
        }
        self.vault_of.insert(plan.site_id.clone(), plan.vault_id.clone());
        let spent = plan
            .vault_funding
            .0
            .saturating_add(plan.site_funding.0)
            .saturating_add(SITE_STORAGE_DEPOSIT);
        self.refund(&plan.payer, plan.deposit.0.saturating_sub(spent));
    }

    /// Private tail of `mark_dark`/`clear_dark`.
    #[private]
    pub fn dark_callback(
        &mut self,
        vault_id: AccountId,
        method: String,
        #[callback_result] transitioned: Result<(), PromiseError>,
    ) {
        require!(
            transitioned.is_ok(),
            format!("DarkTransitionFailed: {method} on {vault_id}")
        );
    }

    /// Private tail of `withdraw_reserve`.
    #[private]
    pub fn reserve_callback(
        &mut self,
        vault_id: AccountId,
        #[callback_result] withdrawn: Result<(), PromiseError>,
    ) {
        require!(
            withdrawn.is_ok(),
            format!("ReserveWithdrawFailed: {vault_id}")
        );
    }
}

impl Contract {
    fn assert_owner(&self) {
        OwnerTrait::assert_owner(self);
    }

    fn assert_attestor(&self) {
        match &self.attestor {
            Some(attestor) => require!(
                env::predecessor_account_id() == *attestor,
                "NotAttestor"
            ),
            None => self.assert_owner(),
        }
    }

    fn create_site_inner(
        &mut self,
        owner_id: AccountId,
        payer: AccountId,
        site_id: AccountId,
        settlement_token: Option<AccountId>,
        config: SiteConfig,
        keys: Vec<String>,
        floors: Vec<U128>,
    ) -> Promise {
        require!(!self.site_code.is_empty(), "SiteCodeNotStored");
        self.require_direct_child(&site_id);
        let min_slot_floor = self.min_slot_floor_for(settlement_token.as_ref());
        let args = self.site_args(
            &owner_id,
            settlement_token,
            &self.protocol_treasury,
            None,
            false,
            &config,
            &keys,
            &floors,
            min_slot_floor,
        );
        let args_len = args.len() as u64;
        let required = self.get_required(args_len).as_yoctonear();
        let attached = env::attached_deposit().as_yoctonear();
        require!(attached >= required, "InsufficientDeposit");
        let funding = attached
            .checked_sub(CREATE_RESERVE)
            .unwrap_or_else(|| env::panic_str("InsufficientDeposit"));

        Promise::new(site_id.clone())
            .create_account()
            .transfer(NearToken::from_yoctonear(funding))
            .deploy_contract(self.site_code.clone())
            .function_call(
                "new".to_owned(),
                args,
                NearToken::from_yoctonear(0),
                Gas::from_tgas(60),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "create_callback".to_owned(),
                    json!({
                        "site_id": site_id,
                        "owner_id": owner_id,
                        "payer": payer,
                        "deposit": U128(attached),
                    })
                    .to_string()
                    .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(80),
                ),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_escrowed_site_inner(
        &mut self,
        owner_id: AccountId,
        payer: AccountId,
        vault_id: AccountId,
        site_id: AccountId,
        settlement_token: Option<AccountId>,
        config: SiteConfig,
        keys: Vec<String>,
        floors: Vec<U128>,
    ) -> Promise {
        require!(!self.site_code.is_empty(), "SiteCodeNotStored");
        require!(!self.escrow_code.is_empty(), "EscrowCodeNotStored");
        require!(self.escrow_params.window_secs != 0, "EscrowParamsNotSet");
        self.require_direct_child(&vault_id);
        self.require_direct_child(&site_id);
        require!(vault_id != site_id, "SameAccount");

        let min_slot_floor = self.min_slot_floor_for(settlement_token.as_ref());
        let site_args = self.site_args(
            &owner_id,
            settlement_token.clone(),
            &self.protocol_treasury,
            Some(vault_id.clone()),
            true,
            &config,
            &keys,
            &floors,
            min_slot_floor,
        );
        let required = self
            .get_required_escrowed(site_args.len() as u64)
            .as_yoctonear();
        let attached = env::attached_deposit().as_yoctonear();
        require!(attached >= required, "InsufficientDeposit");

        let vault_code = (self.escrow_code.len() as u128)
            .checked_mul(STORAGE_BYTE_COST)
            .unwrap_or_else(|| env::panic_str("QuoteOverflow"));
        let vault_funding = vault_code
            .checked_add(ESCROW_STATE_ALLOWANCE)
            .unwrap_or_else(|| env::panic_str("QuoteOverflow"));
        let site_funding = required
            .checked_sub(vault_funding)
            .unwrap_or_else(|| env::panic_str("QuoteOverflow"))
            .checked_sub(SITE_STORAGE_DEPOSIT)
            .unwrap_or_else(|| env::panic_str("QuoteOverflow"))
            .saturating_sub(CREATE_RESERVE);

        let plan = EscrowPlan {
            vault_id: vault_id.clone(),
            site_id,
            payer,
            owner_id,
            settlement_token,
            min_slot_floor,
            config,
            keys,
            floors,
            deposit: U128(attached),
            vault_funding: U128(vault_funding),
            site_funding: U128(site_funding),
        };

        Promise::new(vault_id)
            .create_account()
            .transfer(NearToken::from_yoctonear(vault_funding))
            .deploy_contract(self.escrow_code.clone())
            .function_call(
                "new".to_owned(),
                json!({ "factory": env::current_account_id() })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(30),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "escrowed_vault_callback".to_owned(),
                    serde_json::to_vec(&json!({ "plan": plan })).unwrap_or_else(|_| env::panic_str("PlanEncode")),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(15),
                ),
            )
    }

    /// The site `new` argument object. One builder for both paths so the
    /// escrowed and plain sites can never drift.
    ///
    /// `treasury` is the site's revenue account and `treasury_pinned` freezes
    /// it: an escrowed site passes the vault and `true`, a plain site passes
    /// `null` and `false` (the site then mirrors `protocol_treasury`, exactly
    /// as the origin `initialize` did). `protocol_treasury` stays the protocol
    /// account in both cases, so a site's protocol cut is never routed through
    /// the escrow.
    #[allow(clippy::too_many_arguments)]
    fn site_args(
        &self,
        owner_id: &AccountId,
        settlement_token: Option<AccountId>,
        protocol_treasury: &AccountId,
        treasury: Option<AccountId>,
        treasury_pinned: bool,
        config: &SiteConfig,
        keys: &[String],
        floors: &[U128],
        min_slot_floor: U128,
    ) -> Vec<u8> {
        json!({
            "owner_id": owner_id,
            "protocol_treasury": protocol_treasury,
            "protocol_bps": self.protocol_bps,
            "protocol_rent_bps": self.protocol_rent_bps,
            "settlement_ft": settlement_token,
            "config": config,
            "keys": keys,
            "floors": floors,
            "min_slot_floor": min_slot_floor,
            "treasury": treasury,
            "treasury_pinned": treasury_pinned,
        })
        .to_string()
        .into_bytes()
    }

    fn min_slot_floor_for(&self, settlement_token: Option<&AccountId>) -> U128 {
        let key: AccountId = match settlement_token {
            Some(token) => token.clone(),
            None => NATIVE_TOKEN_KEY
                .parse()
                .unwrap_or_else(|_| env::panic_str("BadNativeKey")),
        };
        self.allowed_tokens
            .get(&key)
            .map(|allowed| allowed.min_slot_floor)
            .unwrap_or_else(|| env::panic_str("TokenNotAllowed"))
    }

    fn require_direct_child(&self, account_id: &AccountId) {
        let current = env::current_account_id();
        let suffix = format!(".{current}");
        require!(
            account_id.as_str().ends_with(&suffix),
            "NotDirectSubAccount"
        );
    }

    fn forward_dark(&self, vault_id: AccountId, method: &str) -> Promise {
        Promise::new(vault_id.clone())
            .function_call(
                method.to_owned(),
                b"{}".to_vec(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(20),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "dark_callback".to_owned(),
                    json!({ "vault_id": vault_id, "method": method })
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(10),
                ),
            )
    }

    fn refund(&self, payer: &AccountId, amount: u128) {
        if amount == 0 {
            return;
        }
        Promise::new(payer.clone())
            .transfer(NearToken::from_yoctonear(amount))
            .detach();
    }
}
