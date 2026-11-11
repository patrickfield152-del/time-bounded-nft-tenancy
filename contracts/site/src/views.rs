//! Read-only surface: the market views, the SDK's one-call `terms`, and the
//! `encumbrance_hash` whose preimage the SDK must reproduce byte for byte.
//! State changes live in `lib.rs`, the FT entry point in `ft_receiver.rs`.
//!
//! `quote` and `encumbrance_hash` are the two reads a buyer needs pinned to
//! the same block as the purchase: `quote` is the price both the SDK and
//! `buy` see, and `encumbrance_hash` is the `expected_terms` value that makes
//! a stale buy revert instead of inheriting tenancy state the buyer never saw.

use near_sdk::json_types::U128;
use near_sdk::{env, near, AccountId};
use near_sdk_contract_tools::nft::Nep171Controller;
use primitive_types::U256;

use crate::pricing;
use crate::{Contract, ContractExt, Listing, Quote, Rental, Slot, TokenId};

/// The site terms the SDK reads in one call (one RPC instead of a dozen).
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermsView {
    pub implementation_version: u32,
    pub take_bps: u64,
    pub payout_bps: u64,
    pub reversion_bps: u64,
    pub max_reversion_weeks: u32,
    pub cooldown_secs: u64,
    pub protocol_bps: u64,
    pub protocol_rent_bps: u64,
    pub site_rent_bps: u64,
    pub min_rent_bps: u64,
    pub max_ask_bps: u64,
    pub floor_delta_bps: u64,
    pub floor_change_cooldown_secs: u64,
    pub min_slot_floor: U128,
    pub max_rental_term_secs: u64,
    /// `None` = native NEAR settlement.
    pub settlement_ft: Option<AccountId>,
    pub paused: bool,
    pub open_registration: bool,
    pub terms_locked: bool,
    pub default_floor: U128,
    pub treasury: AccountId,
    /// Display-only ERC-2981 analog (`treasury_royalty_bps`).
    pub royalty_bps: u16,
}

#[near]
impl Contract {
    /// Origin `quote`: `(effective floor, take price)` for one slot. An
    /// unregistered key quotes zeros, exactly as the origin view read its
    /// zeroed mapping entry.
    pub fn quote(&self, token_id: TokenId) -> Quote {
        let Some(slot) = self.slots.get(&token_id) else {
            return Quote {
                effective_floor: U128(0),
                price: U128(0),
            };
        };
        let take = self.take_quote(slot);
        Quote {
            effective_floor: U128(pricing::to_u128(take.effective_floor, "effective floor")),
            price: U128(pricing::to_u128(take.price, "take price")),
        }
    }

    /// Origin `resolveReversionBase`: the posted ask when there is one,
    /// otherwise what the current owner paid.
    pub fn resolve_reversion_base(&self, token_id: TokenId) -> U128 {
        let Some(slot) = self.slots.get(&token_id) else {
            return U128(0);
        };
        let base = pricing::resolve_reversion_base(
            U256::from(slot.ask_floor.0),
            U256::from(slot.last_price.0),
        );
        U128(pricing::to_u128(base, "reversion base"))
    }

    /// Origin `ownerOfOrZero`, expressed as an `Option` because NEAR has no
    /// zero account to return. `None` = never minted.
    pub fn owner_of_or_zero(&self, token_id: TokenId) -> Option<AccountId> {
        self.token_owner(&token_id)
    }

    /// Raw slot state, or `None` if the key was never registered.
    pub fn slot_of(&self, token_id: TokenId) -> Option<Slot> {
        self.slots.get(&token_id).cloned()
    }

    pub fn registered_slot_count(&self) -> u64 {
        u64::from(self.registered.len())
    }

    /// Origin `registeredSlotsPage`: clamped to the vector, empty past the end.
    pub fn registered_slots_page(&self, start: u64, count: u64) -> Vec<TokenId> {
        let length = u64::from(self.registered.len());
        if start >= length {
            return Vec::new();
        }
        let end = start.saturating_add(count).min(length);
        (start..end)
            .filter_map(|index| u32::try_from(index).ok())
            .filter_map(|index| self.registered.get(index).cloned())
            .collect()
    }

    pub fn listing_of(&self, token_id: TokenId) -> Option<Listing> {
        self.listings.get(&token_id).cloned()
    }

    pub fn rental_of(&self, token_id: TokenId) -> Option<Rental> {
        self.rentals.get(&token_id).cloned()
    }

    pub fn pending_withdrawals(&self, account: AccountId) -> U128 {
        self.pending_withdrawals
            .get(&account)
            .copied()
            .unwrap_or(U128(0))
    }

    pub fn treasury_balance(&self) -> U128 {
        self.treasury_balance
    }

    pub fn implementation_version(&self) -> u32 {
        self.impl_version
    }

    /// One-call terms read for the SDK.
    pub fn terms(&self) -> TermsView {
        TermsView {
            implementation_version: self.impl_version,
            take_bps: self.config.take_bps,
            payout_bps: self.config.payout_bps,
            reversion_bps: self.config.reversion_bps,
            max_reversion_weeks: self.config.max_reversion_weeks,
            cooldown_secs: self.config.cooldown_secs,
            protocol_bps: self.protocol_bps,
            protocol_rent_bps: self.protocol_rent_bps,
            site_rent_bps: self.config.site_rent_bps,
            min_rent_bps: self.config.min_rent_bps,
            max_ask_bps: self.config.max_ask_bps,
            floor_delta_bps: self.config.floor_delta_bps,
            floor_change_cooldown_secs: self.config.floor_change_cooldown_secs,
            min_slot_floor: self.min_slot_floor,
            max_rental_term_secs: self.config.max_rental_term_secs,
            settlement_ft: self.settlement_ft.clone(),
            paused: self.paused,
            open_registration: self.open_registration,
            terms_locked: self.terms_locked,
            default_floor: self.default_floor,
            treasury: self.treasury.clone(),
            royalty_bps: self.treasury_royalty_bps,
        }
    }

    /// Origin `royaltyInfo`: `(treasury, bps * sale_price / 10_000)`,
    /// truncating, and never deducted from a sale. The origin signature
    /// ignored `token_id`, so this one takes only the price.
    pub fn royalty_info(&self, sale_price: U128) -> (AccountId, U128) {
        let royalty = sale_price
            .0
            .checked_mul(u128::from(self.treasury_royalty_bps))
            .unwrap_or_else(|| env::panic_str("MathOverflow"))
            / u128::from(pricing::BPS_DENOMINATOR);
        (self.treasury.clone(), U128(royalty))
    }

    /// Origin `encumbranceHash`: a sha256 over a fixed-width, big-endian
    /// preimage that pins what a buyer inherits.
    ///
    /// Byte layout, in order (all integers big-endian):
    ///
    /// | bytes | field |
    /// |---|---|
    /// | 16 | `base_price` as `u128` |
    /// | 1 | `registered` as `u8` (0 or 1) |
    /// | 4 | tenant length as `u32` (0 when no rental entry exists) |
    /// | N | tenant `AccountId` UTF-8 bytes (N = the length above) |
    /// | 8 | `expiry` as `u64` (nanoseconds, 0 when no rental) |
    /// | 16 | `prepaid` as `u128` |
    /// | 16 | `claimed` as `u128` |
    ///
    /// The length prefix is what makes a variable-width account id
    /// unambiguous; the origin relied on `abi.encode` padding an EVM address to
    /// 32 bytes. The digest is hex-encoded lowercase (64 characters). The SDK
    /// must build the same preimage: no field reordering, no intermediate JSON.
    pub fn encumbrance_hash(&self, token_id: TokenId) -> String {
        let (base_price, registered) = self
            .slots
            .get(&token_id)
            .map(|slot| (slot.base_price.0, u8::from(slot.registered)))
            .unwrap_or((0, 0));
        let (tenant, expiry, prepaid, claimed) = match self.rentals.get(&token_id) {
            Some(rental) => (
                rental.tenant.as_str(),
                rental.expiry,
                rental.prepaid.0,
                rental.claimed.0,
            ),
            None => ("", 0, 0, 0),
        };

        let tenant_bytes = tenant.as_bytes();
        let tenant_length = u32::try_from(tenant_bytes.len())
            .unwrap_or_else(|_| env::panic_str("AccountIdTooLong"));
        let mut preimage = Vec::with_capacity(16 + 1 + 4 + tenant_bytes.len() + 8 + 16 + 16);
        preimage.extend_from_slice(&base_price.to_be_bytes());
        preimage.push(registered);
        preimage.extend_from_slice(&tenant_length.to_be_bytes());
        preimage.extend_from_slice(tenant_bytes);
        preimage.extend_from_slice(&expiry.to_be_bytes());
        preimage.extend_from_slice(&prepaid.to_be_bytes());
        preimage.extend_from_slice(&claimed.to_be_bytes());
        to_hex(&env::sha256(&preimage))
    }
}

/// Lowercase hex, no allocation per byte.
fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
