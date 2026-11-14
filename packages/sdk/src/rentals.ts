/**
 * TypeScript parity twin of `packages/websitekit-contracts/src/RentalsLib.sol` (`pricing.ts`, `pricing.v2-parity.test.ts` / `script/GenV2Vectors.s.sol` via `BigInt` instead of `decimal.js`/`big.js`; `0` truncation).
 */

import { BPS_DENOMINATOR, PricingOverflowError } from './pricing';

/** Seconds in a day. Rent rates are quoted per day; durations are in seconds (§2.5.3). */
export const SECONDS_PER_DAY = 86_400n;

/** The shortest term the contract will open or extend (§2.5.3). */
export const MIN_RENTAL_DURATION = 3_600n;

const U256_MAX = (1n << 256n) - 1n;

/** `a * b`, throwing exactly where Solidity's checked multiply reverts. */
function mul(a: bigint, b: bigint): bigint {
  const result = a * b;
  if (result > U256_MAX) throw new PricingOverflowError('multiply');
  return result;
}

/** `a - b`, throwing exactly where Solidity's checked subtract reverts. */
function sub(a: bigint, b: bigint): bigint {
  if (b > a) throw new PricingOverflowError('subtract');
  return a - b;
}

export interface Accrual {
  /** Rent the owner has earned so far, whether or not it has been claimed. */
  accrued: bigint;
  /**
   * The part of the escrow not yet earned — **what a buyer inherits.**
   * §2.4.2 calls surfacing this the single highest-value client change in the release: without it an
   * encumbered position reads as unbuyable, when it is actually being sold at a discount to the
   * income stream it carries. Effective cost to a buyer is `takePrice - unaccrued`.
   */
  unaccrued: bigint;
}

/** Mirrors `RentalsLib.accruedOf`: linear accrual over `[start, expiry]` (`expiry === 0n` returns `0n`, `end = min(nowTs, expiry)` matches `extendRental`/`endRental`; pass chain `nowTs` not `Date.now()/1000`). */
export function accruedOf(prepaid: bigint, start: bigint, expiry: bigint, nowTs: bigint): bigint {
  if (expiry === 0n) return 0n;
  const end = nowTs < expiry ? nowTs : expiry;
  if (end <= start) return 0n;
  return mul(prepaid, end - start) / (expiry - start);
}

/** Returns `{ accrued, unaccrued }` where `accrued + unaccrued === prepaid` (`prepaid` / `accruedOf`). */
export function accrual(prepaid: bigint, start: bigint, expiry: bigint, nowTs: bigint): Accrual {
  const accrued = accruedOf(prepaid, start, expiry, nowTs);
  return { accrued, unaccrued: sub(prepaid, accrued) };
}

/**
 * Mirrors `RentalsLib.rentCost`. Gross rent for a term.
 * The RATE is per day and the DURATION is in seconds (§2.5.3) — the rate stays per-day because a
 * per-second rate truncates to zero on a 6-decimal token for any cheap position. This is the one
 * place those units meet, and the place where a short term on a cheap position costs literally
 * nothing. That is admissible rather than a bug, so callers must not assume a positive cost.
 */
export function rentCost(ratePerDay: bigint, durationSecs: bigint): bigint {
  return mul(ratePerDay, durationSecs) / SECONDS_PER_DAY;
}

export interface RentSplit {
  /** To the protocol treasury. */
  protocolCut: bigint;
  /** To the site's treasury, at the rate snapshotted on the listing when it was written. */
  siteCut: bigint;
  /** Escrowed, and streamed to whoever owns the position as it accrues. */
  net: bigint;
}

/** Mirrors `RentalsLib.rentSplit` (§2.5, §2.5.1): keys `protocolRentBps + feeBps` (`[1_000, 4_000]`, max `10_000`, throws on `protocolRentBps + feeBps > 10_000`) to `cost` using listing `feeBps` (never `siteRentBps`), with `net` as remainder. */
export function rentSplit(cost: bigint, protocolRentBps: bigint, feeBps: bigint): RentSplit {
  const protocolCut = mul(cost, protocolRentBps) / BPS_DENOMINATOR;
  const siteCut = mul(cost, feeBps) / BPS_DENOMINATOR;
  return { protocolCut, siteCut, net: sub(sub(cost, protocolCut), siteCut) };
}

/**
 * What a tenant pays and where it goes, composed the way `SlotSite.rent` composes it.
 * One call, so a confirm dialog cannot show a cost derived one way and an escrow figure derived
 * another.
 */
export interface RentQuote extends RentSplit {
  /** Gross, and what the tenant is actually charged. */
  cost: bigint;
}

export function quoteRent(
  ratePerDay: bigint,
  durationSecs: bigint,
  protocolRentBps: bigint,
  feeBps: bigint,
): RentQuote {
  const cost = rentCost(ratePerDay, durationSecs);
  return { cost, ...rentSplit(cost, protocolRentBps, feeBps) };
}
