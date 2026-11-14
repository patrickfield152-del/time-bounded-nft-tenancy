//! NEP-141 entry point: `ft_on_transfer` parses JSON `msg` and dispatches `buy`, `rent`, or `extend`.
//! Returns `Value(0)` to keep all, or `Value(n)` to refund; malformed JSON, foreign caller, or unsupported action refunds in full.
//! Market/storage preconditions panic so `ft_resolve_transfer` refunds.
//! Messages (unknown fields ignored):
//! - `{"action":"buy","token_id":String,"max_price":U128,"expected_terms":String,"deadline_ns":u64}`
//! - `{"action":"rent","token_id":String,"duration_secs":u64,"expected_rate_per_day":U128}`
//! - `{"action":"extend","token_id":String,"duration_secs":u64,"expected_rate_per_day":U128}`
//!
//! Tenancy paths (`extend_rental`) call `rent_inner`/`extend_inner` and refund overpayment via `Value`.

use near_sdk::json_types::U128;
use near_sdk::{AccountId, PromiseOrValue, env, near};
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
                let (Some(token_id), Some(max_price), Some(expected_terms), Some(deadline_ns)) =
                    (token_id, max_price, expected_terms, deadline_ns)
                else {
                    return PromiseOrValue::Value(amount);
                };
                // The FT amount is the budget; a buy never defers on this path
                // because the recipient must already cover its own storage.
                match self.settle_buy(crate::BuyOrder {
                    recipient: sender_id,
                    token_id,
                    max_price,
                    expected_terms,
                    deadline_ns,
                    charged: amount,
                    is_ft: true,
                }) {
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
