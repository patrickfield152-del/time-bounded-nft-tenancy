/**
 * Settlement units: exact `U128` decimal strings (`toU128`, `fromU128`), `parseUnits`/`formatUnits`, and `readSettlementAsset` (`ft_metadata` or 24-decimal NEAR vs 6-decimal FT, using `terms()` / `NEAR_FT_ACCOUNT`).
 */
import { readView, type AccountId, type Reader } from './near';
import { nearToYocto } from 'near-api-js';
export { nearToYocto };

/** A `u128` in its wire form: a decimal string between `0` and `2^128 - 1`. */
export type U128 = string;

/** Base units as `bigint` or `U128` wire string (never `number`, which rounds `10^24` past `Number.MAX_SAFE_INTEGER`). */
export type Amount = U128 | bigint;

const U128_MAX = (1n << 128n) - 1n;

const DECIMAL_PATTERN = /^([+-]?)(\d+)(?:\.(\d+))?$/;

/** Encodes integer/bigint/string base units as `U128` (rejects `0.1` fractions). */
export function toU128(value: bigint | string | number): U128 {
  let parsed: bigint;
  if (typeof value === 'bigint') {
    parsed = value;
  } else if (typeof value === 'number') {
    if (!Number.isFinite(value) || !Number.isInteger(value)) {
      throw new RangeError(`stele/settlement: ${value} is not an integer amount`);
    }
    parsed = BigInt(value);
  } else {
    if (!/^[+-]?\d+$/.test(value.trim())) {
      throw new RangeError(`stele/settlement: "${value}" is not an integer amount`);
    }
    parsed = BigInt(value.trim());
  }
  if (parsed < 0n) throw new RangeError(`stele/settlement: ${parsed} is negative`);
  if (parsed > U128_MAX) throw new RangeError(`stele/settlement: ${parsed} exceeds u128`);
  return parsed.toString();
}

/** Parses a wire amount. Throws rather than returning `NaN`. */
export function fromU128(value: U128): bigint {
  if (!/^\d+$/.test(value)) {
    throw new RangeError(`stele/settlement: "${value}" is not a u128 decimal string`);
  }
  return BigInt(value);
}

/** Parses decimal string into base units without floats (`parseUnits('1.5', 6)` = `1_500_000n`; avoids `0.29 * 1e18` -> `290000000000000000` float drift at 18 decimals). */
export function parseUnits(amount: string, decimals: number): bigint {
  if (!Number.isInteger(decimals) || decimals < 0 || decimals > 36) {
    throw new RangeError(`stele/settlement: ${decimals} is not a decimal count in 0..36`);
  }
  const match = DECIMAL_PATTERN.exec(amount.trim());
  if (match === null) throw new RangeError(`stele/settlement: "${amount}" is not a decimal amount`);
  const sign = match[1] === '-' ? -1n : 1n;
  const whole = match[2] ?? '0';
  const fraction = match[3] ?? '';
  if (fraction.length > decimals) {
    throw new RangeError(
      `stele/settlement: "${amount}" has ${fraction.length} fractional digits, more than ${decimals}`,
    );
  }
  const base = BigInt(`${whole}${fraction.padEnd(decimals, '0')}`);
  return sign * base;
}

/** Integer base units back to a decimal string, trimming trailing zeros in the fraction. */
export function formatUnits(value: bigint, decimals: number): string {
  if (!Number.isInteger(decimals) || decimals < 0 || decimals > 36) {
    throw new RangeError(`stele/settlement: ${decimals} is not a decimal count in 0..36`);
  }
  const negative = value < 0n;
  const abs = negative ? -value : value;
  const divisor = 10n ** BigInt(decimals);
  const whole = abs / divisor;
  const fraction = abs % divisor;
  const sign = negative ? '-' : '';
  if (decimals === 0 || fraction === 0n) return `${sign}${whole}`;
  const padded = fraction.toString().padStart(decimals, '0').replace(/0+$/, '');
  return `${sign}${whole}.${padded}`;
}



/** NEP-145 `storage_balance_bounds().min` on a site (`0.007 NEAR`), attached as top-up to `storage_deposit` and refunded on overpay. */
export const MIN_STORAGE_TOP_UP: bigint = nearToYocto('0.007');

/** What a site's prices are counted in. `accountId: null` means native NEAR. */
export interface SettlementAsset {
  accountId: AccountId | null;
  symbol: string;
  decimals: number;
}

/** Native NEAR: 24 decimals, no token account. */
export const NATIVE_NEAR: SettlementAsset = { accountId: null, symbol: 'NEAR', decimals: 24 };

/** `true` when the site settles in native NEAR rather than a NEP-141 token. */
export function isNativeSettlement(asset: Pick<SettlementAsset, 'accountId'> | AccountId | null): boolean {
  if (asset === null || typeof asset === 'string') return asset === null;
  return asset.accountId === null;
}

/** The subset of a NEP-148 response this package reads. */
interface FtMetadataView {
  symbol: string;
  decimals: number;
}

/** Reads NEP-148 `ft_metadata` (`new` cannot await async reads for `min_slot_floor`) or returns `NATIVE_NEAR` when `tokenAccount` is `null`. */
export async function readSettlementAsset(
  reader: Reader,
  tokenAccount: AccountId | null,
): Promise<SettlementAsset> {
  if (tokenAccount === null) return NATIVE_NEAR;
  const metadata = await readView<FtMetadataView>(reader, tokenAccount, 'ft_metadata');
  if (!Number.isInteger(metadata.decimals) || metadata.decimals < 0 || metadata.decimals > 36) {
    throw new RangeError(
      `stele/settlement: ${tokenAccount}.ft_metadata returned ${metadata.decimals} decimals`,
    );
  }
  return { accountId: tokenAccount, symbol: metadata.symbol, decimals: metadata.decimals };
}
