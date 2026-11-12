//! NEP-141 entry point: the settlement FT calls `ft_on_transfer`, this
//! contract parses the JSON message and dispatches.
//!
//! The return value is the NEP-141 refund amount: `Value(0)` means "keep the
//! whole transfer", `Value(n)` means "return n to the sender". A malformed
//! message, a foreign caller or an unsupported action refunds in full. Market
//! guards on a well-formed request (paused, tenancy state, rate, duration,
//! deadline, terms, registration, availability, cooldown, slippage) panic with
//! their specific message, and the FT's `ft_resolve_transfer` then refunds the
//! sender; the storage precondition is a hard panic because a NEP-141 transfer
//! cannot pay NEAR storage.
//!
//! Supported messages (unknown fields are ignored so a later action can add
//! arguments without breaking older senders):
//!
//! - `{"action":"buy","token_id":String,"max_price":U128,
//!    "expected_terms":String,"deadline_ns":u64}`
//! - `{"action":"rent","token_id":String,"duration_secs":u64,
//!    "expected_rate_per_day":U128}`
//! - `{"action":"extend","token_id":String,"duration_secs":u64,
//!    "expected_rate_per_day":U128}`
//!
//! The tenancy actions run the same guards and bookings as the native
//! `rent`/`extend_rental` through `rent_inner`/`extend_inner`; only the funding
//! source differs, and an overpayment is returned as the `Value` refund.

use near_sdk::json_types::U128;
use near_sdk::{env, near, AccountId, PromiseOrValue};
use serde::Deserialize;

use crate::{Contract, ContractExt};

/// The NEP-141 `msg` shape. Every action reads only its own fields; a missing
/// one refunds the whole transfer rather than guessing.
#[derive(Deserialize)]
struct FtMessage {
    action: String,
    token_id: Option<String>,
    max_price: Option<U128>,
    expected_terms: Option<String>,
    deadline_ns: Option<u64>,
    duration_secs: Option<u64>,
    expected_rate_per_day: Option<U128>,
}

#[near]
impl Contract {
    /// Dispatches one FT transfer as a `buy`, `rent` or `extend`.
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

        let Ok(FtMessage {
            action,
            token_id,
            max_price,
            expected_terms,
            deadline_ns,
            duration_secs,
            expected_rate_per_day,
        }) = near_sdk::serde_json::from_str::<FtMessage>(&msg)
        else {
            return PromiseOrValue::Value(amount);
        };

        match action.as_str() {
            "buy" => {
                let (Some(token_id), Some(max_price), Some(expected_terms), Some(deadline_ns)) = (
                    token_id,
                    max_price,
                    expected_terms,
                    deadline_ns,
                ) else {
                    return PromiseOrValue::Value(amount);
                };
                // The FT amount is the budget; a buy never defers on this path
                // because the recipient must already cover its own storage.
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
                    PromiseOrValue::Promise(_) => env::panic_str("FtSettlementCannotDefer"),
                }
            }
            "rent" | "extend" => {
                let (Some(token_id), Some(duration_secs), Some(expected_rate_per_day)) =
                    (token_id, duration_secs, expected_rate_per_day)
                else {
                    return PromiseOrValue::Value(amount);
                };
                // The sender is both the payer and the tenant: a tenancy is
                // opened by the account whose tokens were transferred.
                let refund = if action == "rent" {
                    self.rent_inner(
                        &sender_id,
                        &sender_id,
                        token_id,
                        duration_secs,
                        expected_rate_per_day,
                        amount.0,
                    )
                } else {
                    self.extend_inner(
                        &sender_id,
                        &sender_id,
                        token_id,
                        duration_secs,
                        expected_rate_per_day,
                        amount.0,
                    )
                };
                PromiseOrValue::Value(refund)
            }
            _ => PromiseOrValue::Value(amount),
        }
    }
}
