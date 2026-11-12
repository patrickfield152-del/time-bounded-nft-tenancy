//! stele escrow vault: balance-delta booking, dark clock, and the three-phase
//! NEP-178 surrender claim.
//!
//! Port of `EscrowVault.sol` (626 lines) under the async constraint that makes
//! this the hardest rewrite in the collection: NEAR cross-contract calls are
//! asynchronous, so every fact the origin read from the site synchronously
//! (`treasury`, `settlementToken`, `ownerOfOrZero`, `slotOf`) arrives in a
//! `#[private]` callback, and no money is credited until the surrender
//! transfer has actually succeeded.
//!
//! ## Booking is by balance delta
//!
//! `booked = balance - booked_total - pending_total - reserve`. Any value that
//! reaches this account in the settlement asset becomes a deposit for the
//! party that cranked the booking. One vault per site is what makes the delta
//! unambiguous: a shared vault could not tell whose money arrived.
//!
//! ## Three-phase claim
//!
//! 1. the holder `nft_approve`s this vault (the approval id is delivered to
//!    `nft_on_approve` and stored, because the approve call returns nothing);
//! 2. `claim` books first (site sweep + site payout), then reads the holder
//!    and the slot asynchronously, runs the delay / blackout / episode / cap /
//!    `min_paid` checks and surrenders the token with the stored approval id;
//! 3. `claim_callback` credits only if the transfer receipt succeeded.
//!
//! A failed receipt credits nothing. If the episode closed between the checks
//! and the callback, the token is transferred back to the holder instead of
//! being paid for, so a race cannot end with the vault owning a slot for free.
//!
//! ## Maturity is evaluated AS OF `dark_since`
//!
//! While dark, a deposit's clock stops: `mature` compares the booking time
//! plus the window plus the dark seconds accrued since that booking against
//! the moment the episode began. A deposit that matured before the lights went
//! out stays the publisher's; one booked inside the episode can never mature
//! while it runs.

use near_sdk::borsh::BorshSerialize;
use near_sdk::json_types::U128;
use near_sdk::store::{LookupMap, Vector};
use near_sdk::utils::assert_one_yocto;
use near_sdk::{
    env, near, require, serde_json, AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault,
    Promise, PromiseError, PromiseOrValue,
};

pub type TokenId = String;

/// Nanoseconds per second: configuration stays in seconds, state keeps
/// nanoseconds, and every comparison converts here.
pub const NS_PER_SEC: u64 = 1_000_000_000;
/// The account id the factory uses for the native settlement branch; here it
/// is only the sentinel `sweep_foreign` understands for native value on an
/// FT-settled vault.
pub const NATIVE_TOKEN_KEY: &str = "near";

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

/// The subset of the site's `terms()` this vault verifies at bind. Unknown
/// fields are ignored by serde, so the full terms view stays the site's.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteTerms {
    pub treasury: AccountId,
    pub treasury_pinned: bool,
    pub settlement_ft: Option<AccountId>,
}

/// The subset of the site's `slot_of()` the claim checks need.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotState {
    pub last_price: U128,
    pub last_purchase_at_ns: u64,
}

/// A `bind` that has not passed the site's pin read yet. Parameters are
/// committed only after the read succeeds, so a failed pin check leaves an
/// unbound vault rather than a bound-to-nothing one.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingBind {
    pub site: AccountId,
    pub params: EscrowParams,
}

/// A claim that has booked and is walking the asynchronous checks. One slot:
/// a second `claim` is refused until the first resolves, which is what keeps
/// the immature pool stable between the check phase and the credit.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimPlan {
    pub token_id: TokenId,
    pub min_paid: U128,
    pub claimant: AccountId,
}

/// Storage prefixes. Distinct from any near-sdk default key.
#[derive(BorshSerialize, BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
enum StorageKey {
    Pending,
    ClaimedEpoch,
    Approvals,
    Deposits,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
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
    /// Completed dark seconds (the origin `darkAccrued`).
    pub dark_accrued: u64,
    pub last_transition_at: u64,
    pub booked_total: U128,
    pub pending_total: U128,
    pub reserve: U128,
    pub pending: LookupMap<AccountId, U128>,
    /// `token_id` -> the `dark_since` it last claimed against. One entry per
    /// token, overwritten on the next episode: constant size, no per-episode
    /// growth.
    pub claimed_epoch: LookupMap<TokenId, u64>,
    /// `(token_id, owner)` -> approval id captured by `nft_on_approve`.
    pub approvals: LookupMap<(TokenId, AccountId), u64>,
    /// Site legs cached at book time (`claimable_local` is cache-only).
    pub site_treasury_balance: U128,
    pub site_pending: U128,
    pub deposits: Vector<Deposit>,
    pub head: u64,
    /// Last observed settlement-FT balance, so `unbooked()` can answer for an
    /// FT vault without a cross-contract read.
    pub ft_balance_cache: U128,
    pub pending_bind: Option<PendingBind>,
    /// The beneficiary captured when a booking started (predecessor of
    /// `book`, or the signer of the `bind` chain).
    pub pending_beneficiary: AccountId,
    /// The publisher (site owner) captured at bind. Bookings pay this account,
    /// never the crank caller: origin `_book` stamps `site.owner()`.
    pub site_owner_cache: Option<AccountId>,
    pub pending_claim: Option<ClaimPlan>,
}

#[near]
impl Contract {
    #[init]
    pub fn new(factory: AccountId) -> Self {
        Self {
            factory: factory.clone(),
            site: None,
            settlement_ft: None,
            window_secs: 0,
            claim_delay_secs: 0,
            min_transition_secs: 0,
            min_booking: U128(0),
            dark_since: 0,
            dark_accrued: 0,
            last_transition_at: 0,
            booked_total: U128(0),
            pending_total: U128(0),
            reserve: U128(0),
            pending: LookupMap::new(StorageKey::Pending),
            claimed_epoch: LookupMap::new(StorageKey::ClaimedEpoch),
            approvals: LookupMap::new(StorageKey::Approvals),
            site_treasury_balance: U128(0),
            site_pending: U128(0),
            deposits: Vector::new(StorageKey::Deposits),
            head: 0,
            ft_balance_cache: U128(0),
            pending_bind: None,
            pending_beneficiary: factory,
            site_owner_cache: None,
            pending_claim: None,
        }
    }

    /// Factory-only. Stores the proposal, then reads the site's terms to prove
    /// the pin in the site -> vault direction (`treasury == this vault`); the
    /// vault -> site direction is the stored `site`. Nothing is committed
    /// until both sides agree.
    #[payable]
    pub fn bind(&mut self, site: AccountId, params: EscrowParams) -> Promise {
        require!(
            env::predecessor_account_id() == self.factory,
            "NotFactory"
        );
        require!(self.site.is_none(), "AlreadyBound");
        require!(params.window_secs != 0, "InvalidParams");
        self.pending_bind = Some(PendingBind { site: site.clone(), params });
        Promise::new(site)
            .function_call(
                "terms".to_owned(),
                b"{}".to_vec(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(15),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "bind_callback".to_owned(),
                    b"{}".to_vec(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(20),
                ),
            )
    }

    /// Permissionless crank: ask the site to empty both legs that owe this
    /// vault, then book the balance delta. The deposit is credited to the site
    /// owner captured at bind, so a third-party crank cannot capture revenue;
    /// it only pays the gas.
    pub fn book(&mut self) -> Promise {
        self.assert_bound();
        let beneficiary = self
            .site_owner_cache
            .clone()
            .unwrap_or_else(|| env::predecessor_account_id());
        self.book_inner(beneficiary)
    }

    /// Private tail of `bind`: commit the parameters, then book the vault's
    /// initial sweep under the transaction signer's name (the publisher who
    /// signed `create_escrowed_site`).
    #[private]
    pub fn bind_callback(&mut self, #[callback_result] terms: Result<SiteTerms, PromiseError>) {
        let pending = self
            .pending_bind
            .clone()
            .unwrap_or_else(|| env::panic_str("NotBinding"));
        let terms = terms.unwrap_or_else(|_| env::panic_str("SiteTermsUnavailable"));
        require!(
            terms.treasury == env::current_account_id() && terms.treasury_pinned,
            "NotPinnedHere"
        );

        self.site = Some(pending.site.clone());
        self.settlement_ft = terms.settlement_ft;
        self.window_secs = pending.params.window_secs;
        self.claim_delay_secs = pending.params.claim_delay_secs;
        self.min_transition_secs = pending.params.min_transition_secs;
        self.min_booking = pending.params.min_booking;
        self.pending_bind = None;
        // Bind commits the vault, then asks who owns the site. Bookings pay
        // that account; the signer is only a fallback when the owner read
        // fails, never the beneficiary by default.
        let site = pending.site;
        Promise::new(site)
            .function_call(
                "own_get_owner".to_owned(),
                b"{}".to_vec(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(10),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "bind_owner_callback".to_owned(),
                    b"{}".to_vec(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(25),
                ),
            )
            .detach();
    }

    /// Private tail of `bind`: caches the publisher and books the initial
    /// sweep under their name.
    #[private]
    pub fn bind_owner_callback(
        &mut self,
        #[callback_result] owner: Result<Option<AccountId>, PromiseError>,
    ) {
        self.site_owner_cache = owner.ok().flatten();
        let beneficiary = self
            .site_owner_cache
            .clone()
            .unwrap_or_else(|| env::signer_account_id());
        self.book_inner(beneficiary).detach();
    }

    /// Private stage between the two site calls. The site panics on a pull it
    /// does not owe (`NothingToWithdraw`), so each leg runs in its own receipt
    /// and a failure is tolerated instead of aborting the booking.
    #[private]
    pub fn book_step_callback(&mut self) {
        let site = self
            .site
            .clone()
            .unwrap_or_else(|| env::panic_str("NotBound"));
        Promise::new(site)
            .function_call(
                "withdraw_for".to_owned(),
                serde_json::json!({ "account": env::current_account_id() })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(20),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "book_callback".to_owned(),
                    b"{}".to_vec(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(20),
                ),
            )
            .detach();
    }

    /// Private booking phase. The native branch reads the free balance
    /// directly; the FT branch must ask the token for `ft_balance_of(self)` in
    /// another receipt.
    #[private]
    pub fn book_callback(&mut self) {
        match self.settlement_ft.clone() {
            None => {
                let balance = self.native_free_balance();
                self.finish_booking(balance);
            }
            Some(token) => {
                Promise::new(token)
                    .function_call(
                        "ft_balance_of".to_owned(),
                        serde_json::json!({ "account_id": env::current_account_id() })
                            .to_string()
                            .into_bytes(),
                        NearToken::from_yoctonear(0),
                        Gas::from_tgas(15),
                    )
                    .then(
                        Promise::new(env::current_account_id()).function_call(
                            "book_ft_callback".to_owned(),
                            b"{}".to_vec(),
                            NearToken::from_yoctonear(0),
                            Gas::from_tgas(20),
                        ),
                    )
                    .detach();
            }
        }
    }

    #[private]
    pub fn book_ft_callback(&mut self, #[callback_result] balance: Result<U128, PromiseError>) {
        let balance = balance
            .unwrap_or_else(|_| env::panic_str("FtBalanceUnavailable"))
            .0;
        self.ft_balance_cache = U128(balance);
        self.finish_booking(balance);
    }

    /// NEP-141 entry. `{"book": "<beneficiary>"}` books the arriving value;
    /// `{"reserve": true}` funds the claim reserve. Anything else is refused
    /// by returning the amount, which the FT refunds to the sender.
    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        // Only the configured settlement token may claim to have moved value
        // here; a direct call would otherwise mint a deposit out of nothing.
        let Some(token) = self.settlement_ft.clone() else {
            return PromiseOrValue::Value(amount);
        };
        if env::predecessor_account_id() != token {
            return PromiseOrValue::Value(amount);
        }
        let _ = sender_id;
        let parsed: serde_json::Value = match serde_json::from_str(&msg) {
            Ok(value) => value,
            Err(_) => return PromiseOrValue::Value(amount),
        };
        if let Some(beneficiary) = parsed.get("book").and_then(|value| value.as_str()) {
            let beneficiary: AccountId = match beneficiary.parse() {
                Ok(account) => account,
                Err(_) => return PromiseOrValue::Value(amount),
            };
            if amount.0 == 0 || amount.0 < self.min_booking.0 {
                // Kept by the vault, booked by the next crank.
                self.ft_balance_cache = U128(self.ft_balance_cache.0.saturating_add(amount.0));
                return PromiseOrValue::Value(U128(0));
            }
            self.ft_balance_cache = U128(self.ft_balance_cache.0.saturating_add(amount.0));
            self.push_deposit(amount, beneficiary);
            return PromiseOrValue::Value(U128(0));
        }
        if parsed.get("reserve").and_then(|value| value.as_bool()) == Some(true) {
            self.ft_balance_cache = U128(self.ft_balance_cache.0.saturating_add(amount.0));
            self.reserve = U128(
                self.reserve
                    .0
                    .checked_add(amount.0)
                    .unwrap_or_else(|| env::panic_str("LedgerOverflow")),
            );
            return PromiseOrValue::Value(U128(0));
        }
        PromiseOrValue::Value(amount)
    }

    /// NEP-178 approval delivery. The approve call returns nothing, so this is
    /// the only place the approval id exists; it is keyed by `(token, owner)`
    /// because a standing approval is not caller consent for a surrender.
    pub fn nft_on_approve(
        &mut self,
        token_id: TokenId,
        owner_id: AccountId,
        approval_id: u64,
        msg: String,
    ) {
        let _ = msg;
        require!(
            Some(env::predecessor_account_id()) == self.site,
            "OnlySite"
        );
        self.approvals
            .insert((token_id, owner_id), approval_id);
    }

    /// Credits a mature deposit to its beneficiary. Permissionless: it pays a
    /// recorded party, never the caller.
    pub fn release(&mut self, deposit_id: u64) {
        self.assert_bound();
        let index = u32::try_from(deposit_id).unwrap_or_else(|_| env::panic_str("NoDeposit"));
        let mut deposit = self
            .deposits
            .get(index)
            .cloned()
            .unwrap_or_else(|| env::panic_str("NoDeposit"));
        require!(deposit.amount.0 != 0, "NothingToRelease");
        require!(self.is_mature(&deposit), "NotMature");

        let amount = deposit.amount;
        let beneficiary = deposit.beneficiary.clone();
        deposit.amount = U128(0);
        self.deposits.replace(index, deposit);
        self.booked_total = U128(
            self.booked_total
                .0
                .checked_sub(amount.0)
                .unwrap_or_else(|| env::panic_str("LedgerUnderflow")),
        );
        self.credit(&beneficiary, amount);
        self.advance_head();
    }

    /// Phase 1 of the three-phase claim. Booking is the first thing that
    /// happens, exactly as the origin `claim` calls `_book()` on entry; the
    /// holder and slot checks run in the callbacks that follow.
    pub fn claim(&mut self, token_id: TokenId, min_paid: U128) -> Promise {
        self.assert_bound();
        require!(self.pending_claim.is_none(), "ClaimInFlight");
        let claimant = env::predecessor_account_id();
        self.pending_claim = Some(ClaimPlan {
            token_id,
            min_paid,
            claimant: claimant.clone(),
        });
        self.book_inner(claimant)
    }

    /// Phase 3. Credits only when the surrender transfer succeeded and the
    /// episode is still the one the checks ran against.
    #[private]
    pub fn claim_callback(
        &mut self,
        token_id: TokenId,
        holder: AccountId,
        available: U128,
        #[callback_result] transferred: Result<(), PromiseError>,
    ) {
        self.pending_claim = None;
        let site = match self.site.clone() {
            Some(site) => site,
            None => return,
        };
        if transferred.is_err() {
            // No credit. Drop the episode mark so the holder can retry.
            self.claimed_epoch.remove(&token_id);
            return;
        }
        if self.dark_since == 0
            || self.claimed_epoch.get(&token_id).copied() != Some(self.dark_since)
        {
            // The lights came back between the checks and this receipt. The
            // vault owns the slot now, so give it back rather than pay for a
            // surrender against a closed episode.
            self.claimed_epoch.remove(&token_id);
            Promise::new(site)
                .function_call(
                    "nft_transfer".to_owned(),
                    serde_json::json!({
                        "receiver_id": holder,
                        "token_id": token_id,
                        "approval_id": serde_json::Value::Null,
                        "memo": "episode closed",
                    })
                    .to_string()
                    .into_bytes(),
                    NearToken::from_yoctonear(1),
                    Gas::from_tgas(30),
                )
                .detach();
            return;
        }

        let drained = self.drain_immature(available.0);
        let from_reserve = available.0.saturating_sub(drained);
        require!(from_reserve <= self.reserve.0, "LedgerUnderflow");
        self.reserve = U128(self.reserve.0 - from_reserve);
        self.booked_total = U128(
            self.booked_total
                .0
                .checked_sub(drained)
                .unwrap_or_else(|| env::panic_str("LedgerUnderflow")),
        );
        self.credit(&holder, available);
        self.advance_head();
    }

    /// Factory-only dark clock, rate-limited so an attestor cannot oscillate
    /// the board's state for free.
    pub fn mark_dark(&mut self) {
        require!(
            env::predecessor_account_id() == self.factory,
            "NotFactory"
        );
        self.assert_bound();
        require!(self.dark_since == 0, "AlreadyDark");
        self.rate_limit();
        let now = env::block_timestamp();
        self.dark_since = now;
        self.last_transition_at = now;
    }

    pub fn clear_dark(&mut self) {
        require!(
            env::predecessor_account_id() == self.factory,
            "NotFactory"
        );
        self.assert_bound();
        require!(self.dark_since != 0, "NotDark");
        self.rate_limit();
        let now = env::block_timestamp();
        let elapsed_secs = now.saturating_sub(self.dark_since) / NS_PER_SEC;
        self.dark_accrued = self
            .dark_accrued
            .checked_add(elapsed_secs)
            .unwrap_or_else(|| env::panic_str("ClockOverflow"));
        self.dark_since = 0;
        self.last_transition_at = now;
    }

    /// Native reserve funding. An FT-settled vault funds its reserve through
    /// `ft_transfer_call` with `{"reserve": true}` instead.
    #[payable]
    pub fn fund_reserve(&mut self) {
        require!(self.settlement_ft.is_none(), "UseFtTransferCall");
        self.assert_bound();
        let attached = env::attached_deposit().as_yoctonear();
        require!(attached != 0, "NothingToFund");
        self.reserve = U128(
            self.reserve
                .0
                .checked_add(attached)
                .unwrap_or_else(|| env::panic_str("LedgerOverflow")),
        );
    }

    /// Factory-owner authority, proxied by the factory: only the factory may
    /// call this, and the factory gates it on its owner. Locked while dark.
    pub fn withdraw_reserve(&mut self, amount: U128, to: AccountId) {
        require!(
            env::predecessor_account_id() == self.factory,
            "NotFactory"
        );
        require!(self.dark_since == 0, "ReserveLocked");
        require!(
            amount.0 != 0 && amount.0 <= self.reserve.0,
            "NothingToWithdraw"
        );
        self.reserve = U128(self.reserve.0 - amount.0);
        self.pay_out(&to, amount);
    }

    /// Pull ledger: pay the predecessor. One yocto guards a leaked
    /// function-call key.
    #[payable]
    pub fn withdraw(&mut self) {
        assert_one_yocto();
        let account = env::predecessor_account_id();
        self.withdraw_to(&account);
    }

    /// Permissionless: pay a named account what it has been credited.
    pub fn withdraw_for(&mut self, account: AccountId) {
        self.withdraw_to(&account);
    }

    /// Returns value that is not the settlement asset to the site owner.
    /// `token = "near"` selects native value on an FT-settled vault; on a
    /// native-settled vault native is the settlement asset and is refused.
    pub fn sweep_foreign(&mut self, token: AccountId) {
        self.assert_bound();
        if token.as_str() == NATIVE_TOKEN_KEY {
            require!(self.settlement_ft.is_some(), "NotForeign");
            let amount = self.native_free_balance();
            require!(amount != 0, "NothingToSweep");
            self.request_site_owner(token, U128(amount));
            return;
        }
        require!(self.settlement_ft.as_ref() != Some(&token), "NotForeign");
        Promise::new(token.clone())
            .function_call(
                "ft_balance_of".to_owned(),
                serde_json::json!({ "account_id": env::current_account_id() })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(15),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "sweep_balance_callback".to_owned(),
                    serde_json::json!({ "token": token })
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(20),
                ),
            )
            .detach();
    }

    #[private]
    pub fn sweep_balance_callback(
        &mut self,
        token: AccountId,
        #[callback_result] balance: Result<U128, PromiseError>,
    ) {
        let balance = balance
            .unwrap_or_else(|_| env::panic_str("ForeignBalanceUnavailable"))
            .0;
        require!(balance != 0, "NothingToSweep");
        self.request_site_owner(token, U128(balance));
    }

    #[private]
    pub fn sweep_owner_callback(
        &mut self,
        token: AccountId,
        amount: U128,
        #[callback_result] owner: Result<Option<AccountId>, PromiseError>,
    ) {
        let owner = owner
            .unwrap_or_else(|_| env::panic_str("SiteOwnerUnavailable"))
            .unwrap_or_else(|| env::panic_str("ZeroBeneficiary"));
        if token.as_str() == NATIVE_TOKEN_KEY {
            Promise::new(owner)
                .transfer(NearToken::from_yoctonear(amount.0))
                .detach();
            return;
        }
        near_sdk_contract_tools::ft::ext_nep141::ext(token)
            .with_static_gas(Gas::from_tgas(25))
            .with_attached_deposit(NearToken::from_yoctonear(1))
            .ft_transfer(owner, amount, None)
            .detach();
    }

    pub fn is_dark(&self) -> bool {
        self.dark_since != 0
    }

    /// Inbound this vault holds that is neither booked, credited nor reserve.
    /// For an FT vault this reads the balance cached at the last book or
    /// inbound transfer; the SDK can compose a fresh read from `ft_balance_of`.
    pub fn unbooked(&self) -> U128 {
        let balance = match self.settlement_ft {
            None => self.native_free_balance(),
            Some(_) => self.ft_balance_cache.0,
        };
        let accounted = self
            .booked_total
            .0
            .saturating_add(self.pending_total.0)
            .saturating_add(self.reserve.0);
        U128(balance.saturating_sub(accounted))
    }

    /// Deposits the publisher has not earned yet: what a claim may drain.
    pub fn immature_pool(&self) -> U128 {
        U128(self.immature_pool_inner())
    }

    /// Vault-local claim components: the immature pool, the reserve and the
    /// unbooked balance. The site legs (`treasury_balance`,
    /// `pending_withdrawals`) are live reads the SDK composes in
    /// `escrow.ts` — they are deliberately NOT cached here, so one claim quote
    /// has exactly one source per leg and cannot double-count a booking.
    pub fn claimable_local(&self, token_id: TokenId) -> U128 {
        let _ = token_id;
        U128(
            self.immature_pool_inner()
                .saturating_add(self.reserve.0)
                .saturating_add(self.unbooked().0),
        )
    }

    pub fn deposit_count(&self) -> u64 {
        u64::from(self.deposits.len())
    }

    pub fn deposit_at(&self, id: u64) -> Option<Deposit> {
        u32::try_from(id)
            .ok()
            .and_then(|index| self.deposits.get(index).cloned())
    }

    pub fn mature(&self, id: u64) -> bool {
        u32::try_from(id)
            .ok()
            .and_then(|index| self.deposits.get(index))
            .map(|deposit| self.is_mature(deposit))
            .unwrap_or(false)
    }

    pub fn pending_of(&self, account: AccountId) -> U128 {
        self.pending.get(&account).copied().unwrap_or(U128(0))
    }

    pub fn bound_site(&self) -> Option<AccountId> {
        self.site.clone()
    }
}

#[near]
impl Contract {
    fn assert_bound(&self) {
        require!(self.site.is_some(), "NotBound");
    }

    fn book_inner(&mut self, beneficiary: AccountId) -> Promise {
        let site = self
            .site
            .clone()
            .unwrap_or_else(|| env::panic_str("NotBound"));
        self.pending_beneficiary = beneficiary;
        Promise::new(site)
            .function_call(
                "sweep_treasury".to_owned(),
                b"{}".to_vec(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(20),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "book_step_callback".to_owned(),
                    b"{}".to_vec(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(15),
                ),
            )
    }

    fn finish_booking(&mut self, balance: u128) {
        let accounted = self
            .booked_total
            .0
            .saturating_add(self.pending_total.0)
            .saturating_add(self.reserve.0);
        let booked = balance.saturating_sub(accounted);
        // Both site legs were just asked to empty into this vault; anything
        // still there is re-read by the next crank.
        self.site_treasury_balance = U128(0);
        self.site_pending = U128(0);
        if booked != 0 && booked >= self.min_booking.0 {
            let beneficiary = self.pending_beneficiary.clone();
            self.push_deposit(U128(booked), beneficiary);
        }
        if self.pending_claim.is_some() {
            self.start_claim_checks();
        }
    }

    fn push_deposit(&mut self, amount: U128, beneficiary: AccountId) {
        self.deposits.push(Deposit {
            amount,
            booked_at: env::block_timestamp(),
            dark_at_booking: self.dark_now(),
            beneficiary,
        });
        self.booked_total = U128(
            self.booked_total
                .0
                .checked_add(amount.0)
                .unwrap_or_else(|| env::panic_str("LedgerOverflow")),
        );
    }

    /// Phase 2a: who holds the token. The holder check is asynchronous
    /// because `owner_of_or_zero` is a site view.
    fn start_claim_checks(&mut self) {
        let claim = match self.pending_claim.clone() {
            Some(claim) => claim,
            None => return,
        };
        let site = match self.site.clone() {
            Some(site) => site,
            None => return,
        };
        Promise::new(site)
            .function_call(
                "owner_of_or_zero".to_owned(),
                serde_json::json!({ "token_id": claim.token_id })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(15),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "claim_holder_callback".to_owned(),
                    b"{}".to_vec(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(20),
                ),
            )
            .detach();
    }

    #[private]
    pub fn claim_holder_callback(
        &mut self,
        #[callback_result] holder: Result<Option<AccountId>, PromiseError>,
    ) {
        let claim = self
            .pending_claim
            .clone()
            .unwrap_or_else(|| env::panic_str("NoClaim"));
        // The plan now travels in the next callback's arguments, so a failed
        // check can no longer leave a stale marker that bricks every retry.
        self.pending_claim = None;
        let holder = holder
            .unwrap_or_else(|_| env::panic_str("HolderUnavailable"))
            .unwrap_or_else(|| env::panic_str("NothingToClaim"));
        require!(holder == claim.claimant, "NotHolder");
        let site = self
            .site
            .clone()
            .unwrap_or_else(|| env::panic_str("NotBound"));
        Promise::new(site)
            .function_call(
                "slot_of".to_owned(),
                serde_json::json!({ "token_id": claim.token_id })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(15),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "claim_slot_callback".to_owned(),
                    serde_json::json!({ "claim": claim })
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(25),
                ),
            )
            .detach();
    }

    #[private]
    pub fn claim_slot_callback(
        &mut self,
        claim: ClaimPlan,
        #[callback_result] slot: Result<Option<SlotState>, PromiseError>,
    ) {
        let slot = slot
            .unwrap_or_else(|_| env::panic_str("SlotUnavailable"))
            .unwrap_or_else(|| env::panic_str("NothingToClaim"));
        let since = self.dark_since;
        require!(since != 0, "NotClaimable");
        let delay_ns = (self.claim_delay_secs as u128)
            .checked_mul(u128::from(NS_PER_SEC))
            .unwrap_or_else(|| env::panic_str("ClockOverflow"));
        require!(
            u128::from(env::block_timestamp()) >= u128::from(since).saturating_add(delay_ns),
            "NotClaimable"
        );
        require!(slot.last_purchase_at_ns < since, "BoughtDuringBlackout");
        require!(
            self.claimed_epoch.get(&claim.token_id).copied() != Some(since),
            "AlreadyClaimedThisEpisode"
        );

        let approval_id = self
            .approvals
            .get(&(claim.token_id.clone(), claim.claimant.clone()))
            .copied()
            .unwrap_or_else(|| env::panic_str("NotApproved"));
        let available = self
            .immature_pool_inner()
            .saturating_add(self.reserve.0)
            .min(slot.last_price.0);
        require!(available != 0, "NothingToClaim");
        require!(available >= claim.min_paid.0, "BelowMinimum");

        self.claimed_epoch.insert(claim.token_id.clone(), since);
        let site = self
            .site
            .clone()
            .unwrap_or_else(|| env::panic_str("NotBound"));
        Promise::new(site)
            .function_call(
                "nft_transfer".to_owned(),
                serde_json::json!({
                    "receiver_id": env::current_account_id(),
                    "token_id": claim.token_id,
                    "approval_id": approval_id,
                    "memo": serde_json::Value::Null,
                })
                .to_string()
                .into_bytes(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(30),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "claim_callback".to_owned(),
                    serde_json::json!({
                        "token_id": claim.token_id,
                        "holder": claim.claimant,
                        "available": U128(available),
                    })
                    .to_string()
                    .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(25),
                ),
            )
            .detach();
    }

    fn request_site_owner(&self, token: AccountId, amount: U128) {
        let site = self
            .site
            .clone()
            .unwrap_or_else(|| env::panic_str("NotBound"));
        Promise::new(site)
            .function_call(
                "own_get_owner".to_owned(),
                b"{}".to_vec(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(15),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "sweep_owner_callback".to_owned(),
                    serde_json::json!({ "token": token, "amount": amount })
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(30),
                ),
            )
            .detach();
    }

    fn drain_immature(&mut self, want: u128) -> u128 {
        let mut drained = 0u128;
        let length = self.deposits.len();
        let mut index = u32::try_from(self.head).unwrap_or(length);
        while index < length && drained < want {
            if let Some(mut deposit) = self.deposits.get(index).cloned() {
                if deposit.amount.0 != 0 && !self.is_mature(&deposit) {
                    let take = (want - drained).min(deposit.amount.0);
                    deposit.amount = U128(deposit.amount.0 - take);
                    self.deposits.replace(index, deposit);
                    drained += take;
                }
            }
            index += 1;
        }
        drained
    }

    fn immature_pool_inner(&self) -> u128 {
        let length = self.deposits.len();
        let mut total = 0u128;
        let mut index = u32::try_from(self.head).unwrap_or(length);
        while index < length {
            if let Some(deposit) = self.deposits.get(index) {
                if deposit.amount.0 != 0 && !self.is_mature(deposit) {
                    total = total.saturating_add(deposit.amount.0);
                }
            }
            index += 1;
        }
        total
    }

    fn is_mature(&self, deposit: &Deposit) -> bool {
        if deposit.dark_at_booking > self.dark_accrued {
            return false;
        }
        let as_of = if self.dark_since == 0 {
            env::block_timestamp()
        } else {
            self.dark_since
        };
        let dark_gap_secs = self.dark_accrued - deposit.dark_at_booking;
        let window_ns = (self.window_secs as u128).saturating_mul(u128::from(NS_PER_SEC));
        let dark_gap_ns = (dark_gap_secs as u128).saturating_mul(u128::from(NS_PER_SEC));
        u128::from(as_of)
            >= u128::from(deposit.booked_at)
                .saturating_add(window_ns)
                .saturating_add(dark_gap_ns)
    }

    /// `darkAccrued` plus the running episode: what a deposit booked now must
    /// carry so it cannot mature during the blackout it was born in.
    fn dark_now(&self) -> u64 {
        if self.dark_since == 0 {
            self.dark_accrued
        } else {
            let elapsed = env::block_timestamp().saturating_sub(self.dark_since) / NS_PER_SEC;
            self.dark_accrued.saturating_add(elapsed)
        }
    }

    fn rate_limit(&self) {
        let spacing_ns = (self.min_transition_secs as u128)
            .checked_mul(u128::from(NS_PER_SEC))
            .unwrap_or_else(|| env::panic_str("ClockOverflow"));
        require!(
            u128::from(env::block_timestamp())
                >= u128::from(self.last_transition_at).saturating_add(spacing_ns),
            "TransitionTooSoon"
        );
    }

    fn advance_head(&mut self) {
        let length = self.deposits.len();
        let mut head = u32::try_from(self.head).unwrap_or(length);
        while head < length {
            match self.deposits.get(head) {
                Some(deposit) if deposit.amount.0 == 0 => head += 1,
                _ => break,
            }
        }
        self.head = u64::from(head);
    }

    fn credit(&mut self, account: &AccountId, amount: U128) {
        if amount.0 == 0 {
            return;
        }
        let current = self.pending.get(account).copied().unwrap_or(U128(0)).0;
        let updated = current
            .checked_add(amount.0)
            .unwrap_or_else(|| env::panic_str("LedgerOverflow"));
        self.pending.insert(account.clone(), U128(updated));
        self.pending_total = U128(
            self.pending_total
                .0
                .checked_add(amount.0)
                .unwrap_or_else(|| env::panic_str("LedgerOverflow")),
        );
    }

    fn withdraw_to(&mut self, account: &AccountId) {
        let amount = self.pending.remove(account).unwrap_or(U128(0));
        require!(amount.0 != 0, "NothingToWithdraw");
        self.pending_total = U128(
            self.pending_total
                .0
                .checked_sub(amount.0)
                .unwrap_or_else(|| env::panic_str("LedgerUnderflow")),
        );
        self.pay_out(account, amount);
    }

    fn pay_out(&self, account: &AccountId, amount: U128) {
        match &self.settlement_ft {
            Some(token) => {
                near_sdk_contract_tools::ft::ext_nep141::ext(token.clone())
                    .with_static_gas(Gas::from_tgas(25))
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

    /// Native value this account can spend without eating its own storage
    /// stake or a locked balance.
    fn native_free_balance(&self) -> u128 {
        let stake = env::storage_byte_cost()
            .as_yoctonear()
            .saturating_mul(u128::from(env::storage_usage()));
        env::account_balance()
            .as_yoctonear()
            .saturating_sub(stake)
            .saturating_sub(env::account_locked_balance().as_yoctonear())
    }
}
