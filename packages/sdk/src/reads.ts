/**
 * The read path — one round of parallel views per page, no indexer and no reader contract.
 *
 * The origin deployed a separate `SlotReader` because a view inside a site can only read that site
 * and its enriched per-slot fields were expensive to compose on chain. On NEAR the equivalent is
 * this module: one `slot_of`, one `quote`, one `owner_of_or_zero`, one `listing_of` and one
 * `rental_of` per key, issued together, so a twelve-slot page is still two round trips (one block
 * for the clock, one wave of views) rather than sixty.
 *
 * **Tenancy liveness is computed here, not asked for.** `rental_of` carries the term the contract
 * stores; whether it is live is `now < expiry`, and `now` is the CHAIN's clock. The same read gives
 * the unaccrued remainder a buyer would inherit, through the parity-tested `accruedOf` twin rather
 * than a second implementation of the accrual formula.
 *
 * **Timestamps are nanoseconds at the contract boundary.** `Slot` and `Rental` keep ns; the pure
 * math in `./rentals` stays in seconds, so the conversion happens exactly once, here.
 */
import { slotKey } from './keys';
import { readClock, readView, type AccountId, type QueryBlock, type Reader } from './near';
import type { SiteEconomics } from './pricing';
import { accruedOf } from './rentals';
import { fromU128, type U128 } from './settlement';

const NS_PER_SECOND = 1_000_000_000n;

/** One slot as a page should render it. */
export interface SlotState {
  /** The dotted key from the site's config, e.g. `hero.headline`. */
  key: string;
  /** The on-chain token id — the same string, carried so a row is self-describing. */
  tokenId: string;
  /** `null` when unclaimed, not an empty string. */
  owner: AccountId | null;
  /** `null` when the slot has never been edited. */
  contentHash: string | null;
  /** The site owner's configured floor, in settlement base units. */
  floor: bigint;
  /** What a first claim costs right now — `max(reverted, floor)`. */
  effectiveFloor: bigint;
  /** What the current owner actually paid. */
  lastPrice: bigint;
  /**
   * The owner's posted ask, or `0n`. §3 of the origin: this is the **reversion base** — the price
   * the slot reverts *from* — not a list price and not what a buyer pays. Cleared on every sale.
   */
  askFloor: bigint;
  /** Chain time of the last purchase, in ns. */
  lastPurchaseNs: bigint;
  /** Bumped on every edit; what a client caches content against. */
  contentVersion: number;
  /**
   * **Total PURCHASES, not takeovers** — a first claim is a buy. `takes === 1` means claimed and
   * never taken from anyone, and the number of actual changes of hands is `max(takes - 1, 0)`.
   */
  takes: number;
  /** False for a key the site owner has not registered. Such a slot cannot be bought. */
  registered: boolean;
  /** The publisher's listing toggle. Only meaningful while unclaimed. */
  isAvailable: boolean;
  /** Convenience for the overwhelmingly common branch. */
  isUnclaimed: boolean;
  /** What THIS buyer would be charged — `price` from the site's own `quote`. */
  charged: bigint;

  // --- tenancy ---

  /** The LIVE tenant, or `null`. An expired-but-uncleared term reads as `null`. */
  tenant: AccountId | null;
  /** Raw expiry in ns. Non-zero on a lapsed-but-uncleared term, so it is not a liveness signal. */
  rentalExpiryNs: bigint;
  /** Escrowed rent not yet earned — **what a buyer inherits**. */
  unaccruedRent: bigint;
  /** `charged - unaccruedRent`, floored at zero. The number a confirm dialog should lead with. */
  netCost: bigint;
  /** True when the inherited rent stream exceeds the purchase price outright. */
  isFreeCarry: boolean;
  /** Whether a live tenancy is in force right now. */
  isRented: boolean;
  /** Listed rent, per DAY, or `0n` when not listed. Durations are in SECONDS. */
  ratePerDay: bigint;
  /** The longest term the owner will accept, in seconds. `0n` when not listed. */
  maxDurationSecs: bigint;
  /** Whether the position can be rented right now. */
  isListed: boolean;
}

/** The site's `Slot` struct, on the wire. */
export interface SlotRecord {
  last_price: U128;
  base_price: U128;
  page_hash: string;
  ask_floor: U128;
  last_purchase_at_ns: number;
  floor_updated_at_ns: number;
  content_version: number;
  takes: number;
  registered: boolean;
  available: boolean;
}

/** The site's `quote` result, on the wire. */
export interface QuoteView {
  effective_floor: U128;
  price: U128;
}

/** The site's `Listing` struct, on the wire. */
export interface ListingRecord {
  rate_per_day: U128;
  max_duration_secs: number;
  fee_bps: number;
}

/** The site's `Rental` struct, on the wire. `start`/`expiry` are nanoseconds. */
export interface RentalRecord {
  tenant: AccountId;
  start: number;
  expiry: number;
  prepaid: U128;
  claimed: U128;
  owner_hash_snapshot: string;
}

/** The site's `terms` result, on the wire. */
export interface TermsView {
  implementation_version: number;
  take_bps: number;
  payout_bps: number;
  reversion_bps: number;
  max_reversion_weeks: number;
  cooldown_secs: number;
  protocol_bps: number;
  protocol_rent_bps: number;
  site_rent_bps: number;
  min_rent_bps: number;
  max_ask_bps: number;
  floor_delta_bps: number;
  floor_change_cooldown_secs: number;
  min_slot_floor: U128;
  max_rental_term_secs: number;
  settlement_ft: AccountId | null;
  paused: boolean;
  open_registration: boolean;
  terms_locked: boolean;
  default_floor: U128;
  treasury: AccountId;
  royalty_bps: number;
}

/**
 * A site's terms: the constants it was cloned under, its economics, and its rental and floor
 * policy.
 *
 * **Cacheable, but no longer immutable.** Before the first claim a publisher may change anything;
 * after it, take economics may only ratchet in the holder-safe direction, and rental terms stay
 * freely mutable both ways (a listing snapshots the fee, which is what makes that safe).
 * `termsLocked` tells you which regime a site is in. Cache for the lifetime of a page, not of a
 * session.
 */
export interface SiteTerms {
  implementationVersion: number;
  takeBps: bigint;
  payoutBps: bigint;
  /** Per-week reversion. `10_000n` is none at all. */
  reversionBps: bigint;
  maxReversionWeeks: bigint;
  cooldownSecs: bigint;
  protocolBps: bigint;
  protocolRentBps: bigint;
  /** The site's rent cut. Freely mutable — but a listing SNAPSHOTS it, so never price a live
   *  listing against this. */
  siteRentBps: bigint;
  minRentBps: bigint;
  maxAskBps: bigint;
  floorDeltaBps: bigint;
  floorChangeCooldownSecs: bigint;
  /** The smallest legal floor, derived from the settlement asset's decimals. */
  minSlotFloor: bigint;
  maxRentalTermSecs: bigint;
  /** `null` means native NEAR. Frozen at deploy — there is no setter, ever. */
  settlementFt: AccountId | null;
  paused: boolean;
  openRegistration: boolean;
  /** Set by the first claim in a site's life, and never unset. Gates the ratchet rules. */
  termsLocked: boolean;
  defaultFloor: bigint;
  treasury: AccountId;
  /** Display-only (an ERC-2981 analog): never deducted from a sale. */
  royaltyBps: bigint;
}

function u64(value: number | string): bigint {
  return typeof value === 'number' ? BigInt(Math.trunc(value)) : BigInt(value);
}

function nsToSeconds(value: bigint): bigint {
  return value / NS_PER_SECOND;
}

/**
 * One slot at a pinned clock. The internal shape every public read funnels through.
 */
async function readSlotAt(
  reader: Reader,
  site: AccountId,
  key: string,
  nowNs: bigint,
  blockQuery?: QueryBlock,
): Promise<SlotState> {
  const tokenId = slotKey(key);
  const record = await readView<SlotRecord | null>(reader, site, 'slot_of', { token_id: tokenId }, blockQuery);

  if (record === null) {
    // A key that was never registered. `quote` is not defined for it, so asking would be an RPC
    // error dressed up as a slot state; the honest answer is a row that says "not registered".
    return {
      key,
      tokenId,
      owner: null,
      contentHash: null,
      floor: 0n,
      effectiveFloor: 0n,
      lastPrice: 0n,
      askFloor: 0n,
      lastPurchaseNs: 0n,
      contentVersion: 0,
      takes: 0,
      registered: false,
      isAvailable: false,
      isUnclaimed: true,
      charged: 0n,
      tenant: null,
      rentalExpiryNs: 0n,
      unaccruedRent: 0n,
      netCost: 0n,
      isFreeCarry: false,
      isRented: false,
      ratePerDay: 0n,
      maxDurationSecs: 0n,
      isListed: false,
    };
  }

  const [quote, owner, listing, rental] = await Promise.all([
    readView<QuoteView>(reader, site, 'quote', { token_id: tokenId }, blockQuery),
    readView<AccountId | null>(reader, site, 'owner_of_or_zero', { token_id: tokenId }, blockQuery),
    readView<ListingRecord | null>(reader, site, 'listing_of', { token_id: tokenId }, blockQuery),
    readView<RentalRecord | null>(reader, site, 'rental_of', { token_id: tokenId }, blockQuery),
  ]);

  const charged = fromU128(quote.price);
  const expiryNs = rental === null ? 0n : u64(rental.expiry);
  const isRented = rental !== null && expiryNs !== 0n && nowNs < expiryNs;

  let unaccruedRent = 0n;
  if (rental !== null && expiryNs !== 0n) {
    const prepaid = fromU128(rental.prepaid);
    const accrued = accruedOf(
      prepaid,
      nsToSeconds(u64(rental.start)),
      nsToSeconds(expiryNs),
      nsToSeconds(nowNs),
    );
    unaccruedRent = prepaid > accrued ? prepaid - accrued : 0n;
  }

  const isFreeCarry = unaccruedRent > charged;
  const ratePerDay = listing === null ? 0n : fromU128(listing.rate_per_day);

  return {
    key,
    tokenId,
    owner,
    contentHash: record.page_hash === '' ? null : record.page_hash,
    floor: fromU128(record.base_price),
    effectiveFloor: fromU128(quote.effective_floor),
    lastPrice: fromU128(record.last_price),
    askFloor: fromU128(record.ask_floor),
    lastPurchaseNs: u64(record.last_purchase_at_ns),
    contentVersion: record.content_version,
    takes: record.takes,
    registered: record.registered,
    isAvailable: record.available,
    isUnclaimed: owner === null,
    charged,
    tenant: isRented && rental !== null ? rental.tenant : null,
    rentalExpiryNs: expiryNs,
    unaccruedRent,
    netCost: isFreeCarry ? 0n : charged - unaccruedRent,
    isFreeCarry,
    isRented,
    ratePerDay,
    maxDurationSecs: listing === null ? 0n : u64(listing.max_duration_secs),
    isListed: listing !== null && ratePerDay > 0n,
  };
}

/**
 * Reads every slot on a page, all keys in one wave.
 *
 * Takes STRING keys and returns them on each row, because the alternative — hand back rows keyed by
 * token id and make the caller re-derive the mapping — is how a render loop ends up doing the same
 * lookup on every frame. Unregistered keys come back as rows with `registered: false` rather than
 * throwing: a page whose config has one key the owner has not registered yet is a normal state.
 */
export async function readSlots(
  reader: Reader,
  site: AccountId,
  keys: readonly string[],
): Promise<SlotState[]> {
  if (keys.length === 0) return [];
  const clock = await readClock(reader);
  return Promise.all(keys.map((key) => readSlotAt(reader, site, key, clock.timestampNs)));
}

/** Single-slot convenience. Prefer `readSlots` for anything rendering more than one. */
export async function readSlot(reader: Reader, site: AccountId, key: string): Promise<SlotState> {
  const clock = await readClock(reader);
  return readSlotAt(reader, site, key, clock.timestampNs);
}

/**
 * The directory read: many sites, in parallel.
 *
 * Results come back in the order the sites were given, each already zipped against its own key
 * list. There is no cross-site call to make — NEAR has no multicall — so this is `readSlots` per
 * board issued together, which is the same wall-clock cost as the slowest board.
 */
export async function readSlotsMulti(
  reader: Reader,
  boards: ReadonlyArray<{ site: AccountId; keys: readonly string[] }>,
): Promise<SlotState[][]> {
  return Promise.all(boards.map((board) => readSlots(reader, board.site, board.keys)));
}

/** A site's terms, from one `terms()` call. */
export async function readSiteTerms(
  reader: Reader,
  site: AccountId,
  blockQuery?: QueryBlock,
): Promise<SiteTerms> {
  const view = await readView<TermsView>(reader, site, 'terms', {}, blockQuery);
  return {
    implementationVersion: view.implementation_version,
    takeBps: u64(view.take_bps),
    payoutBps: u64(view.payout_bps),
    reversionBps: u64(view.reversion_bps),
    maxReversionWeeks: u64(view.max_reversion_weeks),
    cooldownSecs: u64(view.cooldown_secs),
    protocolBps: u64(view.protocol_bps),
    protocolRentBps: u64(view.protocol_rent_bps),
    siteRentBps: u64(view.site_rent_bps),
    minRentBps: u64(view.min_rent_bps),
    maxAskBps: u64(view.max_ask_bps),
    floorDeltaBps: u64(view.floor_delta_bps),
    floorChangeCooldownSecs: u64(view.floor_change_cooldown_secs),
    minSlotFloor: fromU128(view.min_slot_floor),
    maxRentalTermSecs: u64(view.max_rental_term_secs),
    settlementFt: view.settlement_ft,
    paused: view.paused,
    openRegistration: view.open_registration,
    termsLocked: view.terms_locked,
    defaultFloor: fromU128(view.default_floor),
    treasury: view.treasury,
    royaltyBps: u64(view.royalty_bps),
  };
}

/**
 * The subset of a site's terms the pricing twin needs, mapped without hand-copying five bps fields.
 *
 * Worth a helper rather than a spread: the fields are all `bigint`s with similar names, so
 * transposing two of them type-checks, runs, and produces a quote that is merely *wrong* — which is
 * the failure mode the whole parity layer exists to prevent one level down.
 */
export function economicsFromTerms(terms: SiteTerms): SiteEconomics {
  return {
    takeBps: terms.takeBps,
    payoutBps: terms.payoutBps,
    reversionBps: terms.reversionBps,
    maxReversionWeeks: terms.maxReversionWeeks,
    protocolBps: terms.protocolBps,
  };
}

/** Whether a site settles in a NEP-141 rather than native NEAR. */
export function isTokenSettled(terms: Pick<SiteTerms, 'settlementFt'>): boolean {
  return terms.settlementFt !== null;
}

/**
 * The `expected_terms` argument every buy needs.
 *
 * **Always read this in its own step and pass it in.** Never compute it inline in an argument list:
 * it has to reflect the state the buyer was actually shown, and a fresh read at submit time would
 * defeat the guard it exists to be by picking up the very change it should be rejecting.
 *
 * It commits to the TENANCY as well as the floor and registration — the rental tuple is what a
 * buyer inherits, so a term opened between quote and submit invalidates the deal they were shown.
 */
export async function readEncumbrance(
  reader: Reader,
  site: AccountId,
  key: string,
): Promise<string> {
  return readView<string>(reader, site, 'encumbrance_hash', { token_id: slotKey(key) });
}

/**
 * Everything a buy needs, read against one block.
 *
 * Three bugs this exists to prevent, all of which look like the chain misbehaving:
 *
 * 1. **A torn read.** A quote from block N against terms from N+1 means the buyer signs a state
 *    that never existed. Every view here is pinned to the clock's own block.
 * 2. **Wall-clock deadlines.** `buy` compares `deadline_ns` against `block_timestamp()`. A user
 *    whose clock is ten minutes slow gets a rejected purchase with nothing in the UI able to
 *    explain why, so `nowNs` is the chain's clock and `buildBuyFrom` uses it.
 * 3. **An encumbered position reading as unbuyable.** `unaccruedRent` and `netCost` come back on
 *    the same read, so the confirm dialog can say "you pay X, you inherit Y, net Z" from numbers
 *    that all describe one block.
 */
export interface BuyContext {
  slot: SlotState;
  expectedTerms: string;
  /** The chain's clock in ns at `blockHeight` — NOT `Date.now()`. */
  nowNs: bigint;
  blockHeight: number;
}

export async function readBuyContext(
  reader: Reader,
  site: AccountId,
  key: string,
): Promise<BuyContext> {
  const clock = await readClock(reader);
  const pinned: QueryBlock = { blockId: clock.height };

  // Sequential rather than parallel, because `encumbrance_hash` on an unregistered key is the
  // contract's business and may not answer: the registration check has to come first so a caller
  // gets the actionable sentence instead of an RPC error.
  const slot = await readSlotAt(reader, site, key, clock.timestampNs, pinned);

  if (!slot.registered) {
    // An unknown key has no `quote`: the contract auto-registers it at `default_floor` during
    // `settle_buy` when registration is open, and rejects it otherwise. Returning the zero row
    // `readSlots` uses for rendering would build a free buy, so the buy path refuses instead.
    throw new Error(
      `stele/reads: "${key}" is not registered on ${site} — registration is closed until the owner ` +
        'registers the key, and there is no quote to build a purchase from',
    );
  }

  const expectedTerms = await readView<string>(
    reader,
    site,
    'encumbrance_hash',
    { token_id: slotKey(key) },
    pinned,
  );
  return { slot, expectedTerms, nowNs: clock.timestampNs, blockHeight: clock.height };
}

/**
 * A tenancy, straight from the site. `readSlots` already carries the fields a page needs; this is
 * for a rental panel that has to show the whole term.
 */
export interface Rental {
  tenant: AccountId | null;
  startNs: bigint;
  expiryNs: bigint;
  /** Net of the fee split — what actually streams to the owner over the term. */
  prepaid: bigint;
  /** How much of `prepaid` has already been swept into the pull ledger. */
  claimed: bigint;
  /** The owner's content hash, restored by `end_rental` when the term lapses. */
  ownerHashSnapshot: string;
  /** Rent earned so far, from the parity-tested `accruedOf` twin. */
  accrued: bigint;
  /** Rent not yet earned — what a buyer inherits. */
  unaccrued: bigint;
  /** Whether the term is live right now. */
  isActive: boolean;
  /** Whether the term has lapsed but not yet been cleared — see `buildEndRental`. */
  isLapsed: boolean;
}

/**
 * @param nowNs The chain time to judge liveness against. Omit and it is READ FROM THE CHAIN, at the
 *   cost of one extra round trip. It is deliberately not `Date.now()`: liveness is decided by
 *   `block_timestamp()`, and the two clocks are close and never equal.
 */
export async function readRental(
  reader: Reader,
  site: AccountId,
  key: string,
  nowNs?: bigint,
): Promise<Rental> {
  const tokenId = slotKey(key);
  const [record, now] = await Promise.all([
    readView<RentalRecord | null>(reader, site, 'rental_of', { token_id: tokenId }),
    nowNs === undefined ? readClock(reader).then((clock) => clock.timestampNs) : Promise.resolve(nowNs),
  ]);

  if (record === null) {
    return {
      tenant: null,
      startNs: 0n,
      expiryNs: 0n,
      prepaid: 0n,
      claimed: 0n,
      ownerHashSnapshot: '',
      accrued: 0n,
      unaccrued: 0n,
      isActive: false,
      isLapsed: false,
    };
  }

  const startNs = u64(record.start);
  const expiryNs = u64(record.expiry);
  const prepaid = fromU128(record.prepaid);
  const accrued =
    expiryNs === 0n
      ? 0n
      : accruedOf(prepaid, nsToSeconds(startNs), nsToSeconds(expiryNs), nsToSeconds(now));
  const isActive = expiryNs !== 0n && now < expiryNs;

  return {
    tenant: record.tenant,
    startNs,
    expiryNs,
    prepaid,
    claimed: fromU128(record.claimed),
    ownerHashSnapshot: record.owner_hash_snapshot,
    accrued,
    unaccrued: prepaid > accrued ? prepaid - accrued : 0n,
    isActive,
    isLapsed: expiryNs !== 0n && !isActive,
  };
}

/** A position's rental listing. `ratePerDay === 0n` means not listed. */
export interface Listing {
  ratePerDay: bigint;
  maxDurationSecs: bigint;
  /**
   * The site's cut, SNAPSHOTTED when the listing was written. **Price an existing listing against
   * this, never against the site's current `siteRentBps`.**
   */
  feeBps: bigint;
  isListed: boolean;
}

/** `null` when the position is not listed. */
export async function readListing(
  reader: Reader,
  site: AccountId,
  key: string,
): Promise<Listing | null> {
  const record = await readView<ListingRecord | null>(reader, site, 'listing_of', {
    token_id: slotKey(key),
  });
  if (record === null) return null;
  const ratePerDay = fromU128(record.rate_per_day);
  return {
    ratePerDay,
    maxDurationSecs: u64(record.max_duration_secs),
    feeBps: u64(record.fee_bps),
    isListed: ratePerDay > 0n,
  };
}

/**
 * Whether an account may write this slot's content.
 *
 * Read rather than reconstructed from ownership, so the client cannot offer an edit button for a
 * transaction that will revert — and during a live tenancy the TENANT holds the content gate while
 * the owner is locked out entirely. There is one definition of who may write, and it is in the
 * contract.
 */
export async function readCanEdit(
  reader: Reader,
  site: AccountId,
  key: string,
  account: AccountId,
): Promise<boolean> {
  return readView<boolean>(reader, site, 'can_edit', { token_id: slotKey(key), account });
}

/** What an account may withdraw from the site's pull ledger. */
export async function readPendingWithdrawal(
  reader: Reader,
  site: AccountId,
  account: AccountId,
): Promise<bigint> {
  return fromU128(await readView<U128>(reader, site, 'pending_withdrawals', { account }));
}

/** The publisher's cut currently sitting in the site, before anyone sweeps it. */
export async function readTreasuryBalance(reader: Reader, site: AccountId): Promise<bigint> {
  return fromU128(await readView<U128>(reader, site, 'treasury_balance'));
}

/** How many keys the site has registered. */
export async function readRegisteredSlotCount(reader: Reader, site: AccountId): Promise<number> {
  return Number(await readView<number>(reader, site, 'registered_slot_count'));
}

/** One page of registered token ids, in registration order. */
export async function readRegisteredSlotsPage(
  reader: Reader,
  site: AccountId,
  start: number,
  count: number,
): Promise<string[]> {
  if (!Number.isInteger(start) || start < 0) throw new RangeError(`stele/reads: start ${start} is not a page index`);
  if (!Number.isInteger(count) || count <= 0) throw new RangeError(`stele/reads: count ${count} is not a page size`);
  return readView<string[]>(reader, site, 'registered_slots_page', { start, count });
}

/** The display-only royalty for a sale, as `[treasury, amount]`. Never deducted from a buy. */
export async function readRoyalty(
  reader: Reader,
  site: AccountId,
  salePrice: U128,
): Promise<{ treasury: AccountId; amount: bigint }> {
  const [treasury, amount] = await readView<readonly [AccountId, U128]>(reader, site, 'royalty_info', {
    sale_price: salePrice,
  });
  return { treasury, amount: fromU128(amount) };
}
