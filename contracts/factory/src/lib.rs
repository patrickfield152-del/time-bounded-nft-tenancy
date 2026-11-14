//! stele factory: per-site sub-account deployment (`create_account -> transfer -> deploy_contract -> function_call("new")` instead of `delegatecall`), escrow (`escrow_code`) wiring, and attestor registry.
//! Port of origin factory/escrow/attestor (5, 6, 20, 18): `store_site_code`/`store_escrow_code` upload wasm instead of `include_bytes!`; `get_required` quotes `site_code.len() * 1e19` yocto plus allowance.
//! `#[private]` callbacks check `is_promise_success()` / `#[callback_result]` and refund on failure.

use near_sdk::borsh::BorshSerialize;
use near_sdk::json_types::{Base64VecU8, U128};
use near_sdk::store::LookupMap;
use near_sdk::utils::is_promise_success;
use near_sdk::{
    AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault, Promise, PromiseError, env, near,
    require, serde_json::json,
};
use near_sdk_contract_tools::Owner;
use near_sdk_contract_tools::owner::Owner as OwnerTrait;

/// Wire mirror of `stele_site::SiteConfig` (kept local so `near-sdk/__abi-generate` avoids duplicate `__near_abi_*` link symbols).
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

/// Wire mirror of `stele_escrow::EscrowParams`.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscrowParams {
    pub window_secs: u64,
    pub claim_delay_secs: u64,
    pub min_transition_secs: u64,
    pub min_booking: U128,
}

/// Storage price per byte: 1e19 yocto (1 NEAR = 10^24 yocto).
pub const STORAGE_BYTE_COST: u128 = 10_000_000_000_000_000_000;
/// Initial site state allowance: 2,000 bytes.
pub const SITE_STATE_ALLOWANCE: u128 = 20_000_000_000_000_000_000_000;
/// Initial escrow vault state allowance: 1,000 bytes.
pub const ESCROW_STATE_ALLOWANCE: u128 = 10_000_000_000_000_000_000_000;
/// Factory reserve per created site: 100 bytes (`vault_of` + callback storage).
pub const CREATE_RESERVE: u128 = 1_000_000_000_000_000_000_000;
/// Site NEP-145 `storage_balance_bounds.max` (21 mNEAR) attached before `bind`.
pub const SITE_STORAGE_DEPOSIT: u128 = 21_000_000_000_000_000_000_000;
/// Allow-list key for native NEAR (24 decimals).
pub const NATIVE_TOKEN_KEY: &str = "near";
/// Native NEAR decimals, seeded at `new`.
pub const NATIVE_DECIMALS: u8 = 24;

/// Settlement-asset policy: `min_slot_floor = 10^(decimals - 4)`.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowedToken {
    pub decimals: u8,
    pub min_slot_floor: U128,
}

/// Escrowed-site callback state carrying refund accounting across async stages.
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

struct SiteDeploySpec {
    owner_id: AccountId,
    payer: AccountId,
    vault_id: Option<AccountId>,
    site_id: AccountId,
    settlement_token: Option<AccountId>,
    config: SiteConfig,
    keys: Vec<String>,
    floors: Vec<U128>,
}

struct SiteArgsSpec<'a> {
    owner_id: &'a AccountId,
    settlement_token: Option<AccountId>,
    treasury: Option<AccountId>,
    treasury_pinned: bool,
    config: &'a SiteConfig,
    keys: &'a [String],
    floors: &'a [U128],
    min_slot_floor: U128,
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
    /// Deployer becomes owner; `protocol_bps` and `protocol_rent_bps` clamp here and in site `new`.
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
            NATIVE_TOKEN_KEY
                .parse()
                .unwrap_or_else(|_| env::panic_str("BadNativeKey")),
            AllowedToken {
                decimals: NATIVE_DECIMALS,
                min_slot_floor: U128(10u128.pow(20)),
            },
        );
        contract
    }

    /// Owner-only upload of site wasm.
    #[payable]
    pub fn store_site_code(&mut self, code: Base64VecU8) {
        self.only_owner();
        require!(!code.0.is_empty(), "EmptyCode");
        self.site_code = code.0;
    }

    /// Owner-only upload of the escrow wasm.
    #[payable]
    pub fn store_escrow_code(&mut self, code: Base64VecU8) {
        self.only_owner();
        require!(!code.0.is_empty(), "EmptyCode");
        self.escrow_code = code.0;
    }

    /// Owner-only: policy for every vault this factory will create.
    pub fn set_escrow_params(&mut self, params: EscrowParams) {
        self.only_owner();
        require!(params.window_secs != 0, "InvalidEscrowParams");
        self.escrow_params = params;
    }

    /// Owner-only: allow `Some(token)` (`decimals` -> `min_slot_floor = 10^(decimals - 4)`; `100` for 6-decimal FT, `10^20` for 24-decimal NEAR, never `1e14`).
    pub fn allow_token(&mut self, token: AccountId, decimals: u8) {
        self.only_owner();
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

    /// Owner-only attestor rotation (`None` fails open to factory owner).
    pub fn set_attestor(&mut self, attestor: Option<AccountId>) {
        self.only_owner();
        self.attestor = attestor;
    }

    pub fn attestor(&self) -> Option<AccountId> {
        self.attestor.clone()
    }

    pub fn vault_of(&self, site_id: AccountId) -> Option<AccountId> {
        self.vault_of.get(&site_id).cloned()
    }

    /// Quote for a plain site: code stake plus state allowance (`args_len` widens above 2,000 B).
    pub fn get_required(&self, args_len: u64) -> NearToken {
        let code_stake = (self.site_code.len() as u128)
            .checked_mul(STORAGE_BYTE_COST)
            .unwrap_or_else(|| env::panic_str("QuoteOverflow"));
        let args_stake = (args_len as u128)
            .checked_mul(STORAGE_BYTE_COST)
            .unwrap_or_else(|| env::panic_str("QuoteOverflow"));
        NearToken::from_yoctonear(code_stake.saturating_add(SITE_STATE_ALLOWANCE.max(args_stake)))
    }

    /// Quote for `create_escrowed_site`: site + vault + NEP-145 registration.
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

    /// Deploys a site owned by caller (`settlement_token = None` -> native; `Some(token)` -> allow-listed).
    /// Failure rolls back and `create_callback` refunds `attached`; excess forwards to site.
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
        self.deploy_site(SiteDeploySpec {
            owner_id: caller.clone(),
            payer: caller,
            vault_id: None,
            site_id,
            settlement_token,
            config,
            keys,
            floors,
        })
    }

    /// Deploys a site owned by `owner_id`; failure refunds `predecessor`.
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
        self.deploy_site(SiteDeploySpec {
            owner_id,
            payer,
            vault_id: None,
            site_id,
            settlement_token,
            config,
            keys,
            floors,
        })
    }

    /// Deploys vault (`escrow_code`) -> pinned site -> NEP-145 registration -> `bind` in one async chain.
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
        self.deploy_escrowed(SiteDeploySpec {
            owner_id,
            payer,
            vault_id: Some(vault_id),
            site_id,
            settlement_token,
            config,
            keys,
            floors,
        })
    }

    /// Attestor gateway to vault dark clock (`None` falls back to factory owner).
    pub fn mark_dark(&mut self, vault_id: AccountId) -> Promise {
        self.only_attestor();
        self.forward_dark(vault_id, "mark_dark")
    }

    pub fn clear_dark(&mut self, vault_id: AccountId) -> Promise {
        self.only_attestor();
        self.forward_dark(vault_id, "clear_dark")
    }

    /// Factory-owner proxy for `vault.withdraw_reserve` (mirrors `factory.owner()`).
    pub fn withdraw_reserve(
        &mut self,
        vault_id: AccountId,
        amount: U128,
        to: AccountId,
    ) -> Promise {
        self.only_owner();
        Promise::new(vault_id.clone())
            .function_call(
                "withdraw_reserve".to_owned(),
                json!({ "amount": amount, "to": to })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(40),
            )
            .then(Promise::new(env::current_account_id()).function_call(
                "reserve_callback".to_owned(),
                json!({ "vault_id": vault_id }).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(10),
            ))
    }

    /// Private tail of `create_site`/`create_site_for`: refunds payer on failure.
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

    /// Stage 1 tail: creates pinned site after vault succeeds, or refunds all.
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
        let args = self.site_args(SiteArgsSpec {
            owner_id: &plan.owner_id,
            settlement_token: plan.settlement_token.clone(),
            treasury: Some(plan.vault_id.clone()),
            treasury_pinned: true,
            config: &plan.config,
            keys: &plan.keys,
            floors: &plan.floors,
            min_slot_floor: plan.min_slot_floor,
        });
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
                    serde_json::to_vec(&json!({ "plan": plan }))
                        .unwrap_or_else(|_| env::panic_str("PlanEncode")),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(40),
                ),
            )
            .detach();
    }

    /// Stage 2 tail: registers vault (`registration_only = false`, `bounds.max`) or refunds remainder.
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
                    serde_json::to_vec(&json!({ "plan": plan }))
                        .unwrap_or_else(|_| env::panic_str("PlanEncode")),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(40),
                ),
            )
            .detach();
    }

    /// Stage 3 tail: binds vault to site and records `vault_of` on success.
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
                    serde_json::to_vec(&json!({ "plan": plan }))
                        .unwrap_or_else(|_| env::panic_str("PlanEncode")),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(40),
                ),
            )
            .detach();
    }

    /// Stage 4 tail: records `vault_of` and refunds unspent deposit.
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
        self.vault_of
            .insert(plan.site_id.clone(), plan.vault_id.clone());
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
    fn only_owner(&self) {
        OwnerTrait::assert_owner(self);
    }

    fn only_attestor(&self) {
        match &self.attestor {
            Some(attestor) => require!(env::predecessor_account_id() == *attestor, "NotAttestor"),
            None => self.only_owner(),
        }
    }

    fn deploy_site(&mut self, spec: SiteDeploySpec) -> Promise {
        let SiteDeploySpec {
            owner_id,
            payer,
            site_id,
            settlement_token,
            config,
            keys,
            floors,
            ..
        } = spec;
        require!(!self.site_code.is_empty(), "SiteCodeNotStored");
        self.must_be_child(&site_id);
        let min_slot_floor = self.floor_for(settlement_token.as_ref());
        let args = self.site_args(SiteArgsSpec {
            owner_id: &owner_id,
            settlement_token,
            treasury: None,
            treasury_pinned: false,
            config: &config,
            keys: &keys,
            floors: &floors,
            min_slot_floor,
        });
        let required = self.get_required(args.len() as u64).as_yoctonear();
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

    fn deploy_escrowed(&mut self, spec: SiteDeploySpec) -> Promise {
        let SiteDeploySpec {
            owner_id,
            payer,
            vault_id,
            site_id,
            settlement_token,
            config,
            keys,
            floors,
        } = spec;
        let vault_id = vault_id.unwrap_or_else(|| env::panic_str("MissingVault"));
        require!(!self.site_code.is_empty(), "SiteCodeNotStored");
        require!(!self.escrow_code.is_empty(), "EscrowCodeNotStored");
        require!(self.escrow_params.window_secs != 0, "EscrowParamsNotSet");
        self.must_be_child(&vault_id);
        self.must_be_child(&site_id);
        require!(vault_id != site_id, "SameAccount");

        let min_slot_floor = self.floor_for(settlement_token.as_ref());
        let site_args = self.site_args(SiteArgsSpec {
            owner_id: &owner_id,
            settlement_token: settlement_token.clone(),
            treasury: Some(vault_id.clone()),
            treasury_pinned: true,
            config: &config,
            keys: &keys,
            floors: &floors,
            min_slot_floor,
        });
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
                    serde_json::to_vec(&json!({ "plan": plan }))
                        .unwrap_or_else(|_| env::panic_str("PlanEncode")),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(15),
                ),
            )
    }

    /// Site `new` arguments shared by plain (`treasury = null`, `treasury_pinned = false`, `protocol_treasury`, `initialize`, `the`, `site`, `s`) and escrowed (`Some(vault)`, `true`) paths.
    fn site_args(&self, spec: SiteArgsSpec<'_>) -> Vec<u8> {
        json!({
            "owner_id": spec.owner_id,
            "protocol_treasury": &self.protocol_treasury,
            "protocol_bps": self.protocol_bps,
            "protocol_rent_bps": self.protocol_rent_bps,
            "settlement_ft": spec.settlement_token,
            "config": spec.config,
            "keys": spec.keys,
            "floors": spec.floors,
            "min_slot_floor": spec.min_slot_floor,
            "treasury": spec.treasury,
            "treasury_pinned": spec.treasury_pinned,
        })
        .to_string()
        .into_bytes()
    }

    fn floor_for(&self, settlement_token: Option<&AccountId>) -> U128 {
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

    fn must_be_child(&self, account_id: &AccountId) {
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
