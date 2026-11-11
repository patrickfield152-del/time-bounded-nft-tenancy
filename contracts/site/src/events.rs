//! NEP-297 events for the stele port, transcribed 1:1 from the origin
//! Solidity event declarations.
//!
//! The origin emitted these as EVM logs from `SlotSite.sol`, `RentalsLib.sol`
//! (attributed to the site because the library was delegatecalled), and the
//! escrow/factory/attestor contracts. The port keeps one catalog and one
//! `standard = "stele"` so an indexer decodes every stele log with a single
//! schema; `version` is per variant because a variant may evolve on its own.
//!
//! Payload rules:
//! - event names and payload fields are snake_case (`SlotBought` ->
//!   `slot_bought`, `previousOwner` -> `previous_owner`);
//! - EVM addresses become `AccountId`; an origin `address(0)` sentinel becomes
//!   `Option<AccountId>` because NEAR has no zero account (documented per
//!   field);
//! - `uint256` amounts become `U128` and `uint64` timestamps become `U64`, so
//!   JSON keeps exact integers (a nanosecond value exceeds 2^53);
//! - a timestamp field carries a `_ns` suffix because port state is
//!   nanosecond-based, while configuration fields that the origin expressed in
//!   seconds keep their `_secs` suffix and second units.
//!
//! Transcription table (origin name -> port variant, 39 rows):
//!
//! | # | origin file | origin event | port variant |
//! |---|---|---|---|
//! | 1 | SlotSite | SlotBought | `SlotBought` |
//! | 2 | SlotSite | SlotEdited | `SlotEdited` |
//! | 3 | SlotSite | EditorSet | `EditorSet` |
//! | 4 | SlotSite | SlotRegistered | `SlotRegistered` |
//! | 5 | SlotSite | AvailabilitySet | `AvailabilitySet` |
//! | 6 | SlotSite | FloorSet | `FloorSet` |
//! | 7 | SlotSite | AskSet | `AskSet` |
//! | 8 | SlotSite | PayoutWithdrawn | `PayoutWithdrawn` |
//! | 9 | SlotSite | TreasurySet | `TreasurySet` |
//! | 10 | SlotSite | TreasurySwept | `TreasurySwept` |
//! | 11 | SlotSite | PausedSet | `PausedSet` |
//! | 12 | SlotSite | OpenRegistrationSet | `OpenRegistrationSet` |
//! | 13 | SlotSite | BaseTokenURISet | `BaseTokenUriSet` |
//! | 14 | SlotSite | RoyaltySet | `RoyaltySet` |
//! | 15 | SlotSite | SiteInitialized | `SiteInitialized` |
//! | 16 | SlotSite | TermsLocked | `TermsLocked` |
//! | 17 | SlotSite | EconomicsChanged | `EconomicsChanged` |
//! | 18 | SlotSite | RentalTermsChanged | `RentalTermsChanged` |
//! | 19 | SlotSite | RentClaimed | `RentClaimed` |
//! | 20 | RentalsLib | RentalListed | `RentalListed` |
//! | 21 | RentalsLib | SlotRented | `SlotRented` |
//! | 22 | RentalsLib | RentalExtended | `RentalExtended` |
//! | 23 | RentalsLib | RentalEnded | `RentalEnded` |
//! | 24 | RentalsLib | UpdateUser | `TenancyChanged` |
//! | 25 | EscrowVault | Bound | `Bound` |
//! | 26 | EscrowVault | Booked | `Booked` |
//! | 27 | EscrowVault | Released | `Released` |
//! | 28 | EscrowVault | Claimed | `Claimed` |
//! | 29 | EscrowVault | Withdrawn | `Withdrawn` |
//! | 30 | EscrowVault | MarkedDark | `MarkedDark` |
//! | 31 | EscrowVault | ClearedDark | `ClearedDark` |
//! | 32 | EscrowVault | ForeignSwept | `ForeignSwept` |
//! | 33 | EscrowVault | ReserveFunded | `ReserveFunded` |
//! | 34 | EscrowVault | ReserveWithdrawn | `ReserveWithdrawn` |
//! | 35 | EscrowFactory | EscrowedSiteCreated | `EscrowedSiteCreated` |
//! | 36 | EscrowFactory | ParamsSet | `ParamsSet` |
//! | 37 | EscrowFactory | TokenAllowed | `TokenAllowed` |
//! | 38 | SlotFactory | SiteCreated | `SiteCreated` |
//! | 39 | Attestor | AttestorSet | `AttestorSet` |

use near_sdk::json_types::{U128, U64};
use near_sdk::{near, AccountId};

/// Every stele event, one NEP-297 `EVENT_JSON` log per emission.
///
/// The `#[near(event_json(standard = "stele"))]` attribute supplies the
/// `standard` field and converts each variant identifier to snake_case for the
/// `event` field; `#[event_version("1.0.0")]` supplies the per-variant
/// `version`. `.emit()` comes from the generated `EventMetadata` impl.
///
/// The NFT standards' own events (`nft_mint`, `nft_transfer`,
/// `nft_metadata_update`, ...) are emitted by near-sdk-contract-tools with
/// `standard: "nep171"` / `version: "1.2.0"` and are deliberately absent here.
#[near(event_json(standard = "stele"))]
pub enum SteleEvent {
    // ---------------------------------------------------------------- site

    /// Origin `SlotBought`. Every sale, claim or take.
    ///
    /// `previous_owner` is `None` on a claim, where the origin recorded
    /// `address(0)`. `payer` is the account that funded the purchase (the
    /// predecessor), which on a routed `buy_for` is not the buyer.
    #[event_version("1.0.0")]
    SlotBought {
        token_id: String,
        buyer: AccountId,
        previous_owner: Option<AccountId>,
        payer: AccountId,
        price: U128,
        payout: U128,
        timestamp_ns: U64,
    },

    /// Origin `SlotEdited`.
    #[event_version("1.0.0")]
    SlotEdited {
        token_id: String,
        editor: AccountId,
        page_hash: String,
        content_version: u32,
        timestamp_ns: U64,
    },

    /// Origin `EditorSet`. `granted_at_takes` is the slot's take counter at
    /// grant time; a later take invalidates the grant.
    #[event_version("1.0.0")]
    EditorSet {
        token_id: String,
        editor: AccountId,
        grantor: AccountId,
        granted_at_takes: u32,
    },

    /// Origin `SlotRegistered`.
    #[event_version("1.0.0")]
    SlotRegistered {
        token_id: String,
        floor: U128,
        timestamp_ns: U64,
    },

    /// Origin `AvailabilitySet`.
    #[event_version("1.0.0")]
    AvailabilitySet {
        token_id: String,
        available: bool,
    },

    /// Origin `FloorSet`.
    #[event_version("1.0.0")]
    FloorSet {
        token_id: String,
        floor: U128,
        timestamp_ns: U64,
    },

    /// Origin `AskSet`.
    #[event_version("1.0.0")]
    AskSet {
        token_id: String,
        owner: AccountId,
        ask_floor: U128,
        timestamp_ns: U64,
    },

    /// Origin `PayoutWithdrawn` (pull ledger).
    #[event_version("1.0.0")]
    PayoutWithdrawn {
        account: AccountId,
        amount: U128,
    },

    /// Origin `TreasurySet`.
    #[event_version("1.0.0")]
    TreasurySet {
        treasury: AccountId,
    },

    /// Origin `TreasurySwept`.
    #[event_version("1.0.0")]
    TreasurySwept {
        treasury: AccountId,
        amount: U128,
    },

    /// Origin `PausedSet`.
    #[event_version("1.0.0")]
    PausedSet {
        paused: bool,
    },

    /// Origin `OpenRegistrationSet`.
    #[event_version("1.0.0")]
    OpenRegistrationSet {
        open: bool,
        default_floor: U128,
    },

    /// Origin `BaseTokenURISet`. Variant renamed `BaseTokenUriSet` to keep
    /// Rust's identifier casing; the emitted event name is
    /// `base_token_uri_set`.
    #[event_version("1.0.0")]
    BaseTokenUriSet {
        uri: String,
    },

    /// Origin `RoyaltySet`. Display-only ERC-2981 analog.
    #[event_version("1.0.0")]
    RoyaltySet {
        bps: u16,
    },

    /// Origin `SiteInitialized`. `settlement_token` is `None` for native NEAR
    /// (the origin's `address(0)`).
    #[event_version("1.0.0")]
    SiteInitialized {
        owner: AccountId,
        settlement_token: Option<AccountId>,
        take_bps: u64,
        payout_bps: u64,
    },

    /// Origin `TermsLocked`. Emitted once, on the first claim in a site's life.
    #[event_version("1.0.0")]
    TermsLocked,

    /// Origin `EconomicsChanged`.
    #[event_version("1.0.0")]
    EconomicsChanged {
        take_bps: u64,
        payout_bps: u64,
        reversion_bps: u64,
        max_reversion_weeks: u32,
    },

    /// Origin `RentalTermsChanged`.
    #[event_version("1.0.0")]
    RentalTermsChanged {
        site_rent_bps: u64,
        max_rental_term_secs: u64,
        min_rent_bps: u64,
    },

    /// Origin `RentClaimed`. `owner` is the current owner at claim time, never
    /// the caller: the method is permissionless.
    #[event_version("1.0.0")]
    RentClaimed {
        token_id: String,
        owner: AccountId,
        amount: U128,
    },

    // ------------------------------------------------------------- rentals

    /// Origin `RentalListed`. The origin emitted a zero-rate record to signal
    /// a delisting; the port keeps that shape (`rate_per_day = 0`,
    /// `max_duration_secs = 0`).
    #[event_version("1.0.0")]
    RentalListed {
        token_id: String,
        owner: AccountId,
        rate_per_day: U128,
        max_duration_secs: u64,
    },

    /// Origin `SlotRented`.
    #[event_version("1.0.0")]
    SlotRented {
        token_id: String,
        tenant: AccountId,
        expiry_ns: U64,
        cost: U128,
    },

    /// Origin `RentalExtended`.
    #[event_version("1.0.0")]
    RentalExtended {
        token_id: String,
        tenant: AccountId,
        expiry_ns: U64,
        cost: U128,
    },

    /// Origin `RentalEnded`. `restored_hash` is the owner's page hash that
    /// `end_rental` restored.
    #[event_version("1.0.0")]
    RentalEnded {
        token_id: String,
        restored_hash: String,
    },

    /// Origin `UpdateUser` (ERC-4907). Renamed `tenancy_changed` because NEAR
    /// has no ERC-4907 and the event marks every tenancy transition, including
    /// the clear on `end_rental` where `tenant` is `None` (origin
    /// `address(0)`) and `expiry_ns` is `0`.
    #[event_version("1.0.0")]
    TenancyChanged {
        token_id: String,
        tenant: Option<AccountId>,
        expiry_ns: U64,
    },

    // --------------------------------------------------------------- escrow

    /// Origin `Bound` (EscrowVault). Durations stay in seconds, as
    /// configured.
    #[event_version("1.0.0")]
    Bound {
        site: AccountId,
        window_secs: u64,
        claim_delay_secs: u64,
        min_transition_secs: u64,
        min_booking: U128,
    },

    /// Origin `Booked` (EscrowVault). `deposit_id` is the vector index.
    #[event_version("1.0.0")]
    Booked {
        deposit_id: u64,
        beneficiary: AccountId,
        amount: U128,
        dark_at_booking_ns: U64,
    },

    /// Origin `Released` (EscrowVault).
    #[event_version("1.0.0")]
    Released {
        deposit_id: u64,
        beneficiary: AccountId,
        amount: U128,
    },

    /// Origin `Claimed` (EscrowVault surrender).
    #[event_version("1.0.0")]
    Claimed {
        token_id: String,
        holder: AccountId,
        amount: U128,
        from_reserve: U128,
    },

    /// Origin `Withdrawn` (EscrowVault pull ledger).
    #[event_version("1.0.0")]
    Withdrawn {
        account: AccountId,
        amount: U128,
    },

    /// Origin `MarkedDark` (EscrowVault).
    #[event_version("1.0.0")]
    MarkedDark {
        at_ns: U64,
    },

    /// Origin `ClearedDark` (EscrowVault). `dark_accrued_ns` is the dark
    /// duration that matured.
    #[event_version("1.0.0")]
    ClearedDark {
        at_ns: U64,
        dark_accrued_ns: U64,
    },

    /// Origin `ForeignSwept` (EscrowVault).
    #[event_version("1.0.0")]
    ForeignSwept {
        token: AccountId,
        to: AccountId,
        amount: U128,
    },

    /// Origin `ReserveFunded` (EscrowVault). The origin field is named `from`,
    /// a Rust keyword, so the Rust field is `funder` with a serde rename that
    /// keeps the payload key `from`.
    #[event_version("1.0.0")]
    ReserveFunded {
        #[serde(rename = "from")]
        funder: AccountId,
        amount: U128,
    },

    /// Origin `ReserveWithdrawn` (EscrowVault).
    #[event_version("1.0.0")]
    ReserveWithdrawn {
        to: AccountId,
        amount: U128,
    },

    // -------------------------------------------------- escrow factory / site factory / attestor

    /// Origin `EscrowedSiteCreated` (EscrowFactory).
    #[event_version("1.0.0")]
    EscrowedSiteCreated {
        owner: AccountId,
        site: AccountId,
        vault: AccountId,
        creator: AccountId,
    },

    /// Origin `ParamsSet` (EscrowFactory). Durations stay in seconds.
    #[event_version("1.0.0")]
    ParamsSet {
        window_secs: u64,
        claim_delay_secs: u64,
        min_transition_secs: u64,
        min_booking: U128,
    },

    /// Origin `TokenAllowed` (EscrowFactory).
    #[event_version("1.0.0")]
    TokenAllowed {
        token: AccountId,
        allowed: bool,
    },

    /// Origin `SiteCreated` (SlotFactory).
    #[event_version("1.0.0")]
    SiteCreated {
        owner: AccountId,
        site: AccountId,
        creator: AccountId,
    },

    /// Origin `AttestorSet` (Attestor registry).
    #[event_version("1.0.0")]
    AttestorSet {
        attestor: AccountId,
    },
}
