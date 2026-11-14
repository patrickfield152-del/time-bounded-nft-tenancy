/**
 * TypeScript parity twin of `packages/websitekit-contracts/src/Pricing.sol` (`Pricing.sol`, `BigInt`, `BN` via `BigInt(bn.toString())`, iterative `decayed * decayBps / 10_000` not `basePrice * 0.9^n` / `Math.pow`, uint256 bounds instead of `decimal.js`/`big.js`; see `CLAUDE.md`, `0.8`, `1`, `2`, `3`, `3.2`).
 */

export const BPS_DENOMINATOR = 10_000n;
export const SECONDS_PER_WEEK = 604_800n;

const U256_MAX = (1n << 256n) - 1n;

/** Thrown (`RangeError` subclass) where Solidity checked arithmetic overflows or underflows `uint256`. */
export class PricingOverflowError extends RangeError {
  constructor(op: string) {
    super(`websitekit/pricing: ${op} overflows uint256 — the chain would revert here`);
    this.name = 'PricingOverflowError';
  }
}

/** `a * b`, reverting exactly where Solidity's checked multiply does. */
function mul(a: bigint, b: bigint): bigint {
  const result = a * b;
  if (result > U256_MAX) throw new PricingOverflowError('multiply');
  return result;
}

/** `a - b`, throwing on underflow (`-`). */
function sub(a: bigint, b: bigint): bigint {
  if (b > a) throw new PricingOverflowError('subtract');
  return a - b;
}

/** `amount * bps / 10_000` — one guarded multiply, one truncating divide, in that order. */
function bpsOf(amount: bigint, bps: bigint): bigint {
  return mul(amount, bps) / BPS_DENOMINATOR;
}

export interface TakeQuote {
  /** What a taker pays to displace the current owner. */
  price: bigint;
  /** `max(decayed, basePrice)` — what a first claim pays, and what the whole split is keyed to. */
  effectiveFloor: bigint;
}

/** Mirrors `Pricing.computeTakePrice`; `lastPrice === 0n` clamps to `basePrice` via `max()`, `maxDecayWeeks = 52n` matches v1. */
export function computeTakePrice(
  lastPrice: bigint,
  basePrice: bigint,
  elapsedWeeks: bigint,
  decayBps: bigint,
  takeBps: bigint,
  maxDecayWeeks: bigint,
): TakeQuote {
  const clampedWeeks = elapsedWeeks > maxDecayWeeks ? maxDecayWeeks : elapsedWeeks;
  let effectiveFloor: bigint;
  if (clampedWeeks === 0n) {
    effectiveFloor = lastPrice > basePrice ? lastPrice : basePrice;
  } else if (decayBps === BPS_DENOMINATOR) {
    mul(lastPrice, BPS_DENOMINATOR);
    effectiveFloor = lastPrice > basePrice ? lastPrice : basePrice;
  } else if (lastPrice <= basePrice && decayBps <= BPS_DENOMINATOR) {
    mul(lastPrice, decayBps);
    effectiveFloor = basePrice;
  } else if (decayBps === 0n) {
    effectiveFloor = basePrice;
  } else {
    let decayed = mul(lastPrice, decayBps) / BPS_DENOMINATOR;
    if (decayed <= basePrice) {
      effectiveFloor = basePrice;
    } else if (decayBps < BPS_DENOMINATOR) {
      for (let i = 1n; i < clampedWeeks; i++) {
        decayed = (decayed * decayBps) / BPS_DENOMINATOR;
        if (decayed <= basePrice) {
          decayed = basePrice;
          break;
        }
      }
      effectiveFloor = decayed;
    } else {
      for (let i = 1n; i < clampedWeeks; i++) {
        decayed = mul(decayed, decayBps) / BPS_DENOMINATOR;
        if (decayed <= basePrice) {
          decayed = basePrice;
          break;
        }
      }
      effectiveFloor = decayed > basePrice ? decayed : basePrice;
    }
  }
  const price = mul(effectiveFloor, takeBps) / BPS_DENOMINATOR;
  return { price, effectiveFloor };
}

/** Mirrors `Pricing.resolveReversionBase`; `askFloor === 0n` uses `lastPrice`, posted ask clears on sale. */
export function resolveReversionBase(askFloor: bigint, lastPrice: bigint): bigint {
  return askFloor !== 0n ? askFloor : lastPrice;
}

/** Mirrors `Pricing.askCeiling`; `maxAskBps` (`>= 10_000`) anchors to `max(lastPrice, basePrice)` so asks cannot compound. */
export function askCeiling(lastPrice: bigint, basePrice: bigint, maxAskBps: bigint): bigint {
  const anchor = lastPrice > basePrice ? lastPrice : basePrice;
  return bpsOf(anchor, maxAskBps);
}

export interface Split {
  /** The effective floor on a claim, the take price on a take. This is what `lastPrice` records. */
  charged: bigint;
  /** To the displaced owner. `0n` on a claim; there is nobody to pay. */
  payout: bigint;
  /** To the protocol treasury, on every buy including claims. */
  protocolCut: bigint;
  /** Whatever remains, to the site. */
  siteCut: bigint;
}

/** Mirrors `Pricing.computeSplit`; keys payout and protocol cuts to `effectiveFloor` (not `charged`), `siteCut` throws if `takeBps <= payoutBps + protocolBps` (`SlotSite._validateConfig`). */
export function computeSplit(
  effectiveFloor: bigint,
  price: bigint,
  isUnclaimed: boolean,
  payoutBps: bigint,
  protocolBps: bigint,
): Split {
  const charged = isUnclaimed ? effectiveFloor : price;
  const payout = isUnclaimed ? 0n : bpsOf(effectiveFloor, payoutBps);
  const protocolCut = bpsOf(effectiveFloor, protocolBps);
  const siteCut = sub(sub(charged, payout), protocolCut);
  return { charged, payout, protocolCut, siteCut };
}

/** Mirrors `SlotSite._computeElapsedWeeks`; throws on underflow. */
export function computeElapsedWeeks(nowTs: bigint, lastPurchaseTs: bigint): bigint {
  if (nowTs < lastPurchaseTs) {
    throw new RangeError('websitekit/pricing: nowTs must be >= lastPurchaseTs');
  }
  return (nowTs - lastPurchaseTs) / SECONDS_PER_WEEK;
}

/** Site take economics (`reversionBps` maps to `computeTakePrice`'s `decayBps`). */
export interface SiteEconomics {
  takeBps: bigint;
  payoutBps: bigint;
  reversionBps: bigint;
  maxReversionWeeks: bigint;
  protocolBps: bigint;
}

export interface BuyBreakdown extends TakeQuote, Split {}

/** Single-call quote and split matching `SlotSite._buy`. */
export function computeBuyBreakdown(
  lastPrice: bigint,
  basePrice: bigint,
  elapsedWeeks: bigint,
  economics: SiteEconomics,
  isUnclaimed: boolean,
): BuyBreakdown {
  const quote = computeTakePrice(
    lastPrice,
    basePrice,
    elapsedWeeks,
    economics.reversionBps,
    economics.takeBps,
    economics.maxReversionWeeks,
  );
  const split = computeSplit(
    quote.effectiveFloor,
    quote.price,
    isUnclaimed,
    economics.payoutBps,
    economics.protocolBps,
  );
  return { ...quote, ...split };
}
