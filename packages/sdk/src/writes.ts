/**
 * Transaction builders — the write half of the SDK.
 *
 * Every function here returns a `CallRequest` rather than sending anything. The SDK never owns a
 * wallet, never picks a nonce and never decides when to submit; that belongs to the app or to a
 * script. `submit` from `./near` is the script path, `toWalletCall` the browser one.
 *
 * **Two things these builders refuse to guess.**
 *
 * `expectedTerms` is required on every buy, with no bypass sentinel, because it has to reflect the
 * state the buyer was actually shown. A builder that read it fresh at submit time would defeat the
 * guard it exists to be — it would pick up the very change it should be rejecting. Read it with
 * `readBuyContext` (or `readEncumbrance`) in its own step and pass it in.
 *
 * `settlementFt` is required on every builder that moves money, because the two settlement paths
 * are not interchangeable. A native-shaped call against a token site reverts having sent nothing; a
 * token-shaped call against a native site is a transfer to an account that is not a token. Pass
 * `readSiteTerms().settlementFt` through — there is no default, because a default would silently be
 * right for one kind of site and wrong for the other.
 *
 * **The token path is `ft_transfer_call`, not an approval.** NEP-141 has no allowance to set: the
 * buyer calls the token once with the cost as the amount and a JSON `msg`, and the site's
 * `ft_on_transfer` dispatches on `action`. The site sees `sender_id` as the payer, which is why
 * `buy_for` exists only on the native path — the FT message carries no recipient field, and
 * inventing one would be a message the contract does not read.
 */
import { assertContentHash } from './content';
import { slotKey, slotKeys } from './keys';
import { type AccountId, type CallRequest } from './near';
import { toU128, type Amount, type U128 } from './settlement';
import type { BuyContext } from './reads';

/**
 * How long a buy stays valid after it is built. Short, because the whole point of a deadline is
 * that a transaction stuck through a price move should expire rather than land at a price nobody
 * agreed to. Measured in NANOSECONDS against `block_timestamp()`.
 */
export const DEFAULT_DEADLINE_SECS = 300n;

/**
 * Slippage headroom applied to the quoted price when the caller does not supply an explicit
 * `max_price`.
 *
 * Zero would be correct and unusable: the quote is read a block or two before the transaction
 * lands, and any reversion boundary crossed in between changes the number. 1% is enough to absorb
 * ordinary drift and far too little to absorb a take, which costs at least 1.4x.
 */
export const DEFAULT_SLIPPAGE_BPS = 100n;

/** The one-yocto guard most owner-only methods assert, and the only deposit they accept. */
const ONE_YOCTO = '1';
/** Non-payable methods. Sending anything here reverts the call. */
const NO_DEPOSIT = '0';

const NS_PER_SECOND = 1_000_000_000n;
const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);

/** Thrown when a builder would produce a call the chosen settlement path cannot express. */
export class UnsupportedSettlementError extends Error {
  constructor(message: string) {
    super(`stele/writes: ${message}`);
    this.name = 'UnsupportedSettlementError';
  }
}

/** Thrown when a site's economics would be rejected by the contract's clamps. */
export class InvalidEconomicsError extends Error {
  constructor(message: string) {
    super(`stele/writes: ${message}`);
    this.name = 'InvalidEconomicsError';
  }
}

/** A `u64` field of a call: an integer that stays exact as a JSON number. */
function toU64(value: bigint | number, field: string): number {
  const asBig = typeof value === 'bigint' ? value : BigInt(Math.trunc(value));
  if (typeof value === 'number' && !Number.isInteger(value)) {
    throw new RangeError(`stele/writes: ${field} must be an integer, got ${value}`);
  }
  if (asBig < 0n) throw new RangeError(`stele/writes: ${field} must be non-negative`);
  if (asBig > MAX_SAFE) throw new RangeError(`stele/writes: ${field} ${asBig} does not fit a JSON number`);
  return Number(asBig);
}

/**
 * A nanosecond timestamp as the JSON number `deadline_ns` expects.
 *
 * Current epoch nanoseconds exceed `Number.MAX_SAFE_INTEGER`, so the value carries up to ~256 ns of
 * double rounding. That is stated rather than hidden: a deadline is compared against a block
 * timestamp and is minutes away, so 256 ns is eight orders of magnitude below anything that could
 * change the outcome.
 */
function nsToJson(value: bigint): number {
  if (value < 0n) throw new RangeError('stele/writes: a deadline cannot be negative');
  return Number(value);
}

function bpsOf(amount: bigint, bps: bigint): bigint {
  return (amount * bps) / 10_000n;
}

/** `max(charged, charged + slippage)` with the guard the contract would otherwise revert on. */
function resolveMaxPrice(
  charged: Amount,
  maxPrice?: Amount,
  slippageBps: bigint = DEFAULT_SLIPPAGE_BPS,
): U128 {
  const base = BigInt(charged);
  if (base < 0n) throw new RangeError('stele/writes: a quoted price cannot be negative');
  if (slippageBps < 0n) throw new RangeError('stele/writes: slippage cannot be negative');
  const resolved = maxPrice === undefined ? base + bpsOf(base, slippageBps) : BigInt(maxPrice);
  if (resolved < base) {
    throw new RangeError(
      `stele/writes: max_price ${resolved} is below the quoted price ${base} — the contract reverts`,
    );
  }
  return toU128(resolved);
}

/** The `deadline_ns` for a call, from a chain clock or an explicit absolute value. */
function resolveDeadline(nowNs: bigint, deadlineNs?: bigint, deadlineSecs?: bigint): number {
  if (deadlineNs !== undefined) return nsToJson(deadlineNs);
  const span = deadlineSecs ?? DEFAULT_DEADLINE_SECS;
  if (span < 0n) throw new RangeError('stele/writes: a deadline cannot be in the past');
  return nsToJson(nowNs + span * NS_PER_SECOND);
}

/** `ft_transfer_call` args: the token takes 1 yocto and the message carries the intent. */
function ftCall(
  ft: AccountId,
  site: AccountId,
  amount: U128,
  msg: Record<string, unknown>,
): CallRequest {
  return {
    contractId: ft,
    methodName: 'ft_transfer_call',
    args: { receiver_id: site, amount, msg: JSON.stringify(msg) },
    deposit: ONE_YOCTO,
  };
}

// ---------------------------------------------------------------------------
// Buy
// ---------------------------------------------------------------------------

export interface BuildBuyOptions {
  site: AccountId;
  /** The readable dotted key — also the token id. */
  tokenId: string;
  /**
   * What the caller was quoted — `SlotState.charged`, from the site's own `quote`.
   *
   * **Not `netCost`.** The contract charges the gross price; the inherited rent arrives separately
   * as it accrues. Quoting the net figure here underfunds the call and reverts.
   */
  charged: Amount;
  /** `null` for native NEAR, the NEP-141 account otherwise. See the module note. */
  settlementFt: AccountId | null;
  /** A freshly-read `encumbrance_hash(token_id)` for the state the buyer was shown. Required. */
  expectedTerms: string;
  /** The CHAIN's clock in ns, from `readBuyContext`. Deadline arithmetic uses it, not `Date.now()`. */
  nowNs: bigint;
  /** Overrides the derived `charged + slippage`. */
  maxPrice?: Amount;
  /** Basis points of headroom over `charged`. Ignored when `maxPrice` is given. */
  slippageBps?: bigint;
  /** Absolute ns, overriding `deadlineSecs`. */
  deadlineNs?: bigint;
  /** Seconds from `nowNs`. */
  deadlineSecs?: bigint;
  /**
   * Who ends up OWNING the slot. Omit and the payer owns it. Native path only: the FT message has
   * no recipient field, so an FT purchase always mints to the token sender.
   */
  recipient?: AccountId;
  /**
   * Extra yoctoNEAR attached for the recipient's NEP-145 registration, when they are not already
   * funded. The site routes it into the generated `storage_deposit` before minting; a fresh account
   * cannot be minted to without it, so a first purchase into a new wallet attaches
   * `max_price + storageTopUp` and gets the change back.
   *
   * A fresh recipient needs `MIN_STORAGE_TOP_UP` (0.007 NEAR, the site's storage bounds minimum);
   * an existing holder who already covers the token's bytes needs nothing. Read
   * `storage_balance_bounds` and `storage_balance_of` rather than guessing, because zero is right
   * for one and reverts for the other. The FT path cannot pay NEAR storage at all, which is why a
   * token purchase requires a pre-registered recipient.
   */
  storageTopUp?: Amount;
}

/**
 * Builds a `buy` or a `buy_for`.
 *
 * `buy_for` exists because `owner = predecessor` has already killed one implementation in the
 * origin's sibling repo, and it unlocks sponsored purchases, gifting and a cross-site router. On the
 * native path the payer attaches `max_price`; the contract refunds the difference to whoever paid,
 * so a relayer is never out of pocket beyond gas.
 */
export function buildBuy(options: BuildBuyOptions): CallRequest {
  const tokenId = slotKey(options.tokenId);
  const maxPrice = resolveMaxPrice(options.charged, options.maxPrice, options.slippageBps);
  const deadlineNs = resolveDeadline(options.nowNs, options.deadlineNs, options.deadlineSecs);

  const topUp = options.storageTopUp === undefined ? 0n : BigInt(toU128(options.storageTopUp));
  if (options.settlementFt === null) {
    const deposit = toU128(BigInt(maxPrice) + topUp);
    if (options.recipient !== undefined) {
      return {
        contractId: options.site,
        methodName: 'buy_for',
        args: {
          recipient: options.recipient,
          token_id: tokenId,
          max_price: maxPrice,
          expected_terms: options.expectedTerms,
          deadline_ns: deadlineNs,
        },
        deposit,
      };
    }
    return {
      contractId: options.site,
      methodName: 'buy',
      args: {
        token_id: tokenId,
        max_price: maxPrice,
        expected_terms: options.expectedTerms,
        deadline_ns: deadlineNs,
      },
      deposit,
    };
  }

  if (topUp !== 0n) {
    throw new UnsupportedSettlementError(
      'an ft_transfer_call cannot attach NEAR storage, so a token purchase has no storage top-up; ' +
        'register the recipient at the site with storage_deposit first',
    );
  }

  if (options.recipient !== undefined) {
    throw new UnsupportedSettlementError(
      'an FT purchase mints to the token sender, so buy_for is native-only; send the ft_transfer_call ' +
        'from the recipient account, or use the native path with a relayer',
    );
  }

  return ftCall(options.settlementFt, options.site, maxPrice, {
    action: 'buy',
    token_id: tokenId,
    max_price: maxPrice,
    expected_terms: options.expectedTerms,
  });
}

/** `buy_for` with a required recipient. Throws on the FT path — see `buildBuy`. */
export function buildBuyFor(options: BuildBuyOptions & { recipient: AccountId }): CallRequest {
  return buildBuy(options);
}

/**
 * The path to reach for. Takes a `BuyContext` — quote, encumbrance and chain clock all read at one
 * pinned block — so the ways a buy can be built wrong are unavailable by construction.
 *
 * @example
 * const terms = await readSiteTerms(reader, site);
 * const context = await readBuyContext(reader, site, 'hero.headline');
 * const request = buildBuyFrom(site, context, terms.settlementFt);
 */
export function buildBuyFrom(
  site: AccountId,
  context: Pick<BuyContext, 'slot' | 'expectedTerms' | 'nowNs'>,
  settlementFt: AccountId | null,
  options: Pick<
    BuildBuyOptions,
    'maxPrice' | 'slippageBps' | 'deadlineNs' | 'deadlineSecs' | 'recipient' | 'storageTopUp'
  > = {},
): CallRequest {
  return buildBuy({
    site,
    tokenId: context.slot.key,
    charged: toU128(context.slot.charged),
    settlementFt,
    expectedTerms: context.expectedTerms,
    nowNs: context.nowNs,
    ...options,
  });
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

/**
 * Writes a slot's content hash. The bytes have to be stored somewhere retrievable by hash BEFORE
 * this lands: a hash on-chain whose bytes were never uploaded is a permanently blank slot that
 * looks exactly like a bug in the SDK.
 *
 * During a live tenancy the TENANT is the only account this succeeds for; the owner is locked out
 * entirely. Check with `readCanEdit` rather than inferring from ownership.
 *
 * **Non-payable, deliberately**: a function-call key cannot attach a deposit, and `edit` is the one
 * method a tenant's session key is allowed to call.
 */
export function buildEdit(options: { site: AccountId; tokenId: string; pageHash: string }): CallRequest {
  assertContentHash(options.pageHash);
  return {
    contractId: options.site,
    methodName: 'edit',
    args: { token_id: slotKey(options.tokenId), page_hash: options.pageHash },
    deposit: NO_DEPOSIT,
  };
}

/**
 * Grants or revokes delegated editing. The grant dies on its own the first time the slot changes
 * hands, so this does not need pairing with a cleanup call — but it also does not survive a take.
 */
export function buildSetEditor(options: {
  site: AccountId;
  tokenId: string;
  editor: AccountId;
}): CallRequest {
  return {
    contractId: options.site,
    methodName: 'set_editor',
    args: { token_id: slotKey(options.tokenId), editor: options.editor },
    deposit: ONE_YOCTO,
  };
}

// ---------------------------------------------------------------------------
// Rentals
// ---------------------------------------------------------------------------

/**
 * Lists a position for rent. Owner only.
 *
 * `ratePerDay` is per DAY and `maxDurationSecs` is in SECONDS: seconds make the fast-rotation
 * regime expressible at all, while a per-second rate would truncate to zero for any cheap position.
 * A sale clears the listing but NOT an active tenancy — the new owner should not be renting out at
 * a rate they never chose, while the tenant keeps the term they paid for.
 */
export function buildListForRent(options: {
  site: AccountId;
  tokenId: string;
  ratePerDay: Amount;
  maxDurationSecs: bigint | number;
}): CallRequest {
  if (BigInt(options.ratePerDay) < 0n) throw new RangeError('stele/writes: a rent rate cannot be negative');
  return {
    contractId: options.site,
    methodName: 'list_for_rent',
    args: {
      token_id: slotKey(options.tokenId),
      rate_per_day: toU128(options.ratePerDay),
      max_duration_secs: toU64(options.maxDurationSecs, 'max_duration_secs'),
    },
    deposit: ONE_YOCTO,
  };
}

export interface BuildRentOptions {
  site: AccountId;
  tokenId: string;
  /** Seconds. At least one hour, at most the listing's `maxDurationSecs`. */
  durationSecs: bigint | number;
  /**
   * The rate the tenant was shown, from `readListing().ratePerDay`.
   *
   * Required and exact — the contract reverts on any mismatch. This is the rental path's
   * `max_price`: without it an owner front-runs a rental by raising the rate between quote and
   * transaction.
   */
  expectedRatePerDay: Amount;
  /** What the term costs, from `rentCost`/`quoteRent`. */
  cost: Amount;
  settlementFt: AccountId | null;
}

/**
 * Opens a tenancy. Anyone may rent; the position must be owned (there is nobody to pay otherwise)
 * and must not already carry a tenancy — including a lapsed one that has not been cleared, which
 * is what makes `buildEndRental` load-bearing rather than housekeeping.
 */
export function buildRent(options: BuildRentOptions): CallRequest {
  const tokenId = slotKey(options.tokenId);
  const durationSecs = toU64(options.durationSecs, 'duration_secs');
  const args = {
    token_id: tokenId,
    duration_secs: durationSecs,
    expected_rate_per_day: toU128(options.expectedRatePerDay),
  };

  if (options.settlementFt === null) {
    return {
      contractId: options.site,
      methodName: 'rent',
      args,
      deposit: toU128(options.cost),
    };
  }
  return ftCall(options.settlementFt, options.site, toU128(options.cost), { action: 'rent', ...args });
}

/**
 * Extends a live tenancy. Current tenant only, and the term must not have lapsed.
 *
 * Three rules that surprise people: the cap applies to the REMAINING window rather than the total,
 * so no buyer inherits more than `maxRentalTermSecs` however many extensions preceded them; the
 * rate charged is the CURRENT listing's, not the one originally paid; and the unearned remainder of
 * the old term rolls forward rather than being refunded.
 */
export function buildExtendRental(options: BuildRentOptions): CallRequest {
  const tokenId = slotKey(options.tokenId);
  const durationSecs = toU64(options.durationSecs, 'duration_secs');
  const args = {
    token_id: tokenId,
    duration_secs: durationSecs,
    expected_rate_per_day: toU128(options.expectedRatePerDay),
  };

  if (options.settlementFt === null) {
    return {
      contractId: options.site,
      methodName: 'extend_rental',
      args,
      deposit: toU128(options.cost),
    };
  }
  return ftCall(options.settlementFt, options.site, toU128(options.cost), { action: 'extend', ...args });
}

/**
 * Sweeps earned rent into the owner's pull ledger. Permissionless, and always credits the CURRENT
 * owner rather than the caller — so a keeper can run it for a whole directory without knowing who
 * owns what.
 */
export function buildClaimRent(options: { site: AccountId; tokenId: string }): CallRequest {
  return {
    contractId: options.site,
    methodName: 'claim_rent',
    args: { token_id: slotKey(options.tokenId) },
    deposit: NO_DEPOSIT,
  };
}

/**
 * Clears a lapsed tenancy and restores the owner's content hash. Permissionless once the term has
 * expired.
 *
 * **Load-bearing for the mechanism, not just tidiness.** `rent` refuses to open against a
 * lapsed-but-uncleared term, so until someone calls this the position has left the rental market
 * entirely — for everyone, indefinitely — and the expired tenant's content is still on the page.
 */
export function buildEndRental(options: { site: AccountId; tokenId: string }): CallRequest {
  return {
    contractId: options.site,
    methodName: 'end_rental',
    args: { token_id: slotKey(options.tokenId) },
    deposit: NO_DEPOSIT,
  };
}

// ---------------------------------------------------------------------------
// Ledger
// ---------------------------------------------------------------------------

/**
 * Pays an account what the site owes it from the pull ledger. Permissionless: the money can only
 * ever go to `account`, so a relayer can clear a queue without being trusted with it.
 */
export function buildWithdrawFor(options: { site: AccountId; account: AccountId }): CallRequest {
  return {
    contractId: options.site,
    methodName: 'withdraw_for',
    args: { account: options.account },
    deposit: NO_DEPOSIT,
  };
}

/**
 * Moves the publisher's cut from the site into the treasury. Permissionless, and always pays the
 * pinned treasury — which is the only reason an escrow vault can be that treasury at all.
 */
export function buildSweepTreasury(options: { site: AccountId }): CallRequest {
  return {
    contractId: options.site,
    methodName: 'sweep_treasury',
    args: {},
    deposit: NO_DEPOSIT,
  };
}

// ---------------------------------------------------------------------------
// Site creation
// ---------------------------------------------------------------------------

/**
 * The take economics a site starts with. Mutable before the first claim and a one-way ratchet after
 * it — so this is a starting point rather than a permanent choice.
 */
export interface SiteEconomicsConfig {
  /** 1.4x is `14_000`. Must exceed `payoutBps + protocolBps`, and is capped at `30_000`. */
  takeBps: bigint;
  /** 1.15x is `11_500`. Must be at least `10_000` — a site can only monetize the spread. */
  payoutBps: bigint;
  /** Per-week reversion. `9_700` is 0.97; `10_000` is none at all. */
  reversionBps: bigint;
  /** Capped at 52. */
  maxReversionWeeks: bigint;
  /** Minimum seconds between takes on one slot. Capped at 7 days. */
  cooldownSecs: bigint;
}

/** The rental policy. Freely mutable for the site's whole life. */
export interface SiteRentalConfig {
  /** The site's cut of rent. `site_rent_bps + protocol_rent_bps` must land in `[1_000, 4_000]`. */
  siteRentBps: bigint;
  /** The longest term the site will allow, in seconds. One hour to 365 days. */
  maxRentalTermSecs: bigint;
  /** Anti-poisoning rate floor, in bps of a position's effective floor, per day. `1..10_000`. */
  minRentBps: bigint;
}

/** The floor lever and the ask cap. May only tighten after the first claim. */
export interface SiteFloorPolicyConfig {
  /** How far one floor change may move a floor. `0` (frozen) or `1..2_000`. */
  floorDeltaBps: bigint;
  /** Seconds between floor changes. At least 24 hours. */
  floorChangeCooldownSecs: bigint;
  /** Ceiling on an owner's ask, in bps of `max(lastPrice, floor)`. At least `10_000`. */
  maxAskBps: bigint;
}

/**
 * The `SiteConfig` struct a site is initialized with — exactly the eleven fields the contract
 * holds, nothing else.
 *
 * Names, symbols and base URIs are NOT here: `new` sets `ContractMetadata` itself, and only
 * `set_base_uri` can change any of it afterwards. Open registration and the default floor are not
 * init arguments either — a site starts closed with `default_floor = min_slot_floor` — and the
 * display-only royalty starts at zero. Those are owner calls after the site exists, which is why
 * `buildCreateSite` does not pretend to set them.
 */
export interface SiteConfigArgs {
  take_bps: number;
  payout_bps: number;
  reversion_bps: number;
  max_reversion_weeks: number;
  cooldown_secs: number;
  floor_delta_bps: number;
  floor_change_cooldown_secs: number;
  max_ask_bps: number;
  min_rent_bps: number;
  site_rent_bps: number;
  max_rental_term_secs: number;
}

export interface BuildCreateSiteOptions {
  factory: AccountId;
  /** The direct sub-account the factory will create, e.g. `demo.stele.testnet`. */
  siteId: string;
  economics: SiteEconomicsConfig;
  rentals: SiteRentalConfig;
  floorPolicy: SiteFloorPolicyConfig;
  /** Registered in the same batch as the deploy, so one transaction produces a working board. */
  slots?: Readonly<Record<string, Amount>>;
  /**
   * Who owns the SITE. Omit and the sender owns it; supply it and the sender is merely paying.
   */
  owner?: AccountId;
  /**
   * Settlement asset for the site. Omit for native NEAR; pass the mock FT's account id for an
   * FT-settled site. The factory derives `min_slot_floor` from this asset's decimals.
   */
  settlementToken?: AccountId;
  /**
   * Attached deposit in yoctoNEAR. Read `get_required(args)` from the factory: it must cover the
   * new account's state AND its code storage, and only the factory knows the wasm size.
   */
  deposit: Amount;
}

/** A site's set-up without the call that deploys it. */
export type SiteConfigOptions = Pick<
  BuildCreateSiteOptions,
  'economics' | 'rentals' | 'floorPolicy' | 'slots'
>;

const ONE_HOUR = 3_600n;
const ONE_DAY = 86_400n;
const SEVEN_DAYS = 604_800n;
const ONE_YEAR = 31_536_000n;

/**
 * The clamps below are duplicated from the contract on purpose, and this is the one place in the
 * SDK where duplicating a contract rule is right: the contract's version is the authority and
 * reverts, but a revert here costs a deploy transaction and produces an error name rather than a
 * sentence.
 *
 * The protocol legs are NOT checked: they are the factory's state, supplied by its own `new`, and
 * a client that invented them would be validating against numbers the deployment never agreed to.
 * `take_bps > payout_bps + protocol_bps` is therefore enforced where it can be — on chain.
 */
function assertSiteConfig(options: SiteConfigOptions): void {
  const { economics, rentals, floorPolicy } = options;
  const fail = (message: string): never => {
    throw new InvalidEconomicsError(message);
  };

  if (economics.payoutBps < 10_000n) fail('payoutBps must be at least 10_000 so a displaced owner gets their principal back');
  if (economics.takeBps > 30_000n) fail('takeBps is capped at 30_000 (3x)');
  if (economics.takeBps <= economics.payoutBps) fail('takeBps must exceed payoutBps — a site can only monetize the spread');
  if (economics.reversionBps <= 0n || economics.reversionBps > 10_000n) fail('reversionBps must be in (0, 10_000]');
  if (economics.maxReversionWeeks > 52n) fail('maxReversionWeeks is capped at 52');
  if (economics.cooldownSecs > SEVEN_DAYS) fail('cooldownSecs is capped at 7 days');

  if (rentals.siteRentBps > 4_000n) fail('siteRentBps is capped at 4_000; site_rent_bps + protocol_rent_bps stays in [1_000, 4_000]');
  if (rentals.maxRentalTermSecs < ONE_HOUR || rentals.maxRentalTermSecs > ONE_YEAR) fail('maxRentalTermSecs must be in [1 hour, 365 days]');
  if (rentals.minRentBps <= 0n || rentals.minRentBps > 10_000n) fail('minRentBps must be in (0, 10_000]');

  if (floorPolicy.floorDeltaBps !== 0n && (floorPolicy.floorDeltaBps < 1n || floorPolicy.floorDeltaBps > 2_000n)) {
    fail('floorDeltaBps must be 0 (frozen) or in [1, 2_000]');
  }
  if (floorPolicy.floorChangeCooldownSecs < ONE_DAY) fail('floorChangeCooldownSecs must be at least 24 hours');
  if (floorPolicy.maxAskBps < 10_000n) fail('maxAskBps must be at least 10_000 so an ask can always cover the last price');
}

/**
 * The `SiteConfig` struct and the slots' keys and floors, checked against the contract's clamps —
 * the one implementation behind `buildCreateSite`.
 */
export function siteConfigOf(options: SiteConfigOptions): {
  config: SiteConfigArgs;
  keys: string[];
  floors: U128[];
} {
  const { economics, rentals, floorPolicy, slots = {} } = options;
  assertSiteConfig(options);

  const names = Object.keys(slots);
  const keys = slotKeys(names);
  const floors = names.map((name) => toU128(slots[name]!));

  return {
    config: {
      take_bps: toU64(economics.takeBps, 'take_bps'),
      payout_bps: toU64(economics.payoutBps, 'payout_bps'),
      reversion_bps: toU64(economics.reversionBps, 'reversion_bps'),
      max_reversion_weeks: toU64(economics.maxReversionWeeks, 'max_reversion_weeks'),
      cooldown_secs: toU64(economics.cooldownSecs, 'cooldown_secs'),
      floor_delta_bps: toU64(floorPolicy.floorDeltaBps, 'floor_delta_bps'),
      floor_change_cooldown_secs: toU64(floorPolicy.floorChangeCooldownSecs, 'floor_change_cooldown_secs'),
      max_ask_bps: toU64(floorPolicy.maxAskBps, 'max_ask_bps'),
      min_rent_bps: toU64(rentals.minRentBps, 'min_rent_bps'),
      site_rent_bps: toU64(rentals.siteRentBps, 'site_rent_bps'),
      max_rental_term_secs: toU64(rentals.maxRentalTermSecs, 'max_rental_term_secs'),
    },
    keys,
    floors,
  };
}

/**
 * Builds `create_site` or `create_site_for`.
 *
 * The factory creates the account, deploys the site wasm and calls `new` in one batch; the deposit
 * must cover the account's state and its code storage, which is why it is a required argument read
 * from the factory rather than a constant here.
 *
 * Two settings a deployment wants are deliberately absent: `new` starts a site closed, with
 * `default_floor = min_slot_floor` and a zero display royalty. Open registration, the default floor
 * and the royalty are owner calls on the created site (`set_open_registration`,
 * `set_royalty`), and the deploy script issues them in the same batch after the create returns.
 */
export function buildCreateSite(options: BuildCreateSiteOptions): CallRequest {
  const { config, keys, floors } = siteConfigOf(options);
  const args: Record<string, unknown> = {
    site_id: options.siteId,
    settlement_token: options.settlementToken ?? null,
    config,
    keys,
    floors,
  };
  if (options.owner !== undefined) args.owner_id = options.owner;

  return {
    contractId: options.factory,
    methodName: options.owner === undefined ? 'create_site' : 'create_site_for',
    args,
    deposit: toU128(options.deposit),
  };
}
