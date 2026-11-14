/**
 * Wire-number conversions. One definition for the whole SDK.
 * - A JSON number is exact only up to 2^53 - 1. Nanosecond timestamps and
 *   `u64` amounts exceed it. Send them through `toU64` or `nsToJson`.
 * - `u128` travels as a decimal string. `fromU128` and `u64` parse strictly.
 *   A silent `Number()` is the rounding bug these helpers prevent.
 * - The contract keeps seconds in configuration and nanoseconds in state.
 */

/** Nanoseconds per second: the contract's state clock. */
export const NS_PER_SECOND = 1_000_000_000n;

/** `BigInt(Number.MAX_SAFE_INTEGER)`: the largest exact JSON number. */
export const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);

/** A wire `u64` (number or decimal string) as an exact `bigint`. */
export function wireU64(value: number | string): bigint {
  return typeof value === 'number' ? BigInt(Math.trunc(value)) : BigInt(value);
}

/** A `bigint` for JSON. Throws instead of rounding; `field` names the caller. */
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
