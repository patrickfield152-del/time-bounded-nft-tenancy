//! NEP-141 entry point: the settlement FT calls `ft_on_transfer`, this
//! contract parses the JSON message and dispatches.
//!
//! The return value is the NEP-141 refund amount: `Value(0)` means "keep the
//! whole transfer", `Value(n)` means "return n to the sender". A malformed
//! message, a foreign caller or an unsupported action refunds in full. Market
//! guards on a well-formed buy (deadline, terms, registration, availability,
//! cooldown, slippage) panic with their specific message, and the FT's
//! `ft_resolve_transfer` then refunds the sender; the storage precondition is
//! a hard panic because a NEP-141 transfer cannot pay NEAR storage.
//!
//! Wave 2 handles `{"action":"buy", ...}`. Tenancy actions (`rent`,
//! `extend`) arrive with the tenancy methods in wave 3.

use near_sdk::json_types::U128;
use near_sdk::{env, near, AccountId, PromiseOrValue};
use serde::Deserialize;

use crate::{Contract, ContractExt};

/// The NEP-141 `msg` shape for a buy. Unknown fields are ignored so the wave-3
/// tenancy fields can be added without breaking older senders.
#[derive(Deserialize)]
struct FtMessage {
    action: String,
    token_id: Option<String>,
    max_price: Option<U128>,
    expected_terms: Option<String>,
    deadline_ns: Option<u64>,
}

#[near]
impl Contract {
    /// Dispatches one FT transfer. `msg` is
    /// `{"action":"buy","token_id":String,"max_price":U128,
    /// "expected_terms":String,"deadline_ns":u64}`.
    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        // Only the configured settlement token may drive the market spine; any
        // other caller gets the whole transfer back.
        let Some(settlement_ft) = self.settlement_ft.clone() else {
            return PromiseOrValue::Value(amount);
        };
        if env::predecessor_account_id() != settlement_ft {
            return PromiseOrValue::Value(amount);
        }

        let Ok(request) = near_sdk::serde_json::from_str::<FtMessage>(&msg) else {
            return PromiseOrValue::Value(amount);
        };
        if request.action != "buy" {
            return PromiseOrValue::Value(amount);
        }
        let (Some(token_id), Some(max_price), Some(expected_terms), Some(deadline_ns)) = (
            request.token_id,
            request.max_price,
            request.expected_terms,
            request.deadline_ns,
        ) else {
            return PromiseOrValue::Value(amount);
        };

        // The FT amount is the budget; a buy never defers on this path because
        // the recipient must already cover its own storage.
        match self.settle_buy(
            sender_id,
            token_id,
            max_price,
            expected_terms,
            deadline_ns,
            amount,
            true,
        ) {
            PromiseOrValue::Value(refund) => PromiseOrValue::Value(refund),
            PromiseOrValue::Promise(_) => {
                env::panic_str("FtSettlementCannotDefer")
            }
        }
    }
}
