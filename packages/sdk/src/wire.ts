/**
 * Numbers at the boundary — the conversions every wire value takes.
 *
 * Two hazards this module centralizes:
 *
 * 1. **A JSON number is only safe up to 2^53 − 1.** Nanosecond timestamps and
 *    u64 amounts both exceed it, so anything crossing the wire as a number goes
 *    through `toU64`/`nsToJson` and refuses to round.
 * 2. **`u128` rides the wire as a decimal string.** `fromU128` (settlement) and
 *    `u64` here both parse strictly; a silent `Number()` is the rounding bug
 *    every other helper exists to prevent.
 *
 * Seconds-to-nanoseconds conversion lives here too, because the contract keeps
 * configuration in seconds and state in nanoseconds, and both sides of the SDK
 * need the same divisor.
 */

/** Nanoseconds per second: the contract's state clock. */
export const NS_PER_SECOND = 1_000_000_000n;

/** `BigInt(Number.MAX_SAFE_INTEGER)`: the largest exact JSON number. */
export const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);

/**
 * A wire `u64` (JSON number or decimal string) as an exact `bigint`.
 */
export function wireU64(value: number | string): bigint {
  return typeof value === 'number' ? BigInt(Math.trunc(value)) : BigInt(value);
}

/**
 * A `bigint` that must survive as a JSON number. Throws rather than rounding —
 * `field` names the argument so the error points at the builder that lied.
 */
export function jsonU64(value: bigint | number, field: string): number {
  const asBig = typeof value === 'number' ? BigInt(Math.trunc(value)) : value;
  if (asBig < 0n || asBig > MAX_SAFE) {
    throw new RangeError(`stele/wire: ${field} ${asBig} does not fit a JSON number`);
  }
  return Number(asBig);
}

/** Nanoseconds truncated to whole seconds (floor, never round). */
export function nsToSeconds(value: bigint): bigint {
  return value / NS_PER_SECOND;
}
