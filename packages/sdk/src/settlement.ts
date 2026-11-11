/**
 * What a site's money is, and how a money value crosses the wire.
 *
 * **Every amount on the wire is a decimal string.** `u128` in JSON is either a string type
 * (`U128`) or a number, and near-sdk's `U128` is the string form for the reason that matters
 * here: 10^24 yocto per NEAR puts ordinary amounts past `Number.MAX_SAFE_INTEGER`, so a value
 * routed through a JavaScript number is silently rounded. `toU128`/`fromU128` are the only
 * conversions, and they are strict — a negative, fractional or out-of-range value throws instead
 * of becoming a plausible-looking wrong number.
 *
 * **Native NEAR has 24 decimals.** A floor written for a 6-decimal mock FT means something
 * different by 18 orders of magnitude, so nothing here guesses: `readSettlementAsset` reads
 * `ft_metadata` for a token site and returns the fixed native description otherwise, and callers
 * format with the decimals they got back.
 *
 * There is no chain list and no deployed-token table. The settlement account comes from the site's
 * own `terms()` read or from `NEAR_FT_ACCOUNT`, so a testnet deployment and a local sandbox are
 * the same code path.
 */
import { readView, type AccountId, type Reader } from './near';

/** A `u128` in its wire form: a decimal string between `0` and `2^128 - 1`. */
export type U128 = string;

/**
 * What a builder accepts where money is involved: base units as a `bigint` (what every read and
 * every pricing twin returns) or already in wire form. Never a `number` — a `number` is the one
 * representation that rounds.
 */
export type Amount = U128 | bigint;

const U128_MAX = (1n << 128n) - 1n;

const DECIMAL_PATTERN = /^([+-]?)(\d+)(?:\.(\d+))?$/;

/**
 * Narrows a value to `U128`.
 *
 * `number` is accepted only as an integer, because `0.1` has no exact `bigint` and a fractional
 * amount is always a units mistake rather than a rounding preference.
 */
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

/**
 * A human decimal amount into integer base units, exactly.
 *
 * `parseUnits('1.5', 6)` is `1_500_000n`. Never multiplies by a float: the whole point of the
 * strict pattern is that `0.29 * 1e18` is not `290000000000000000` in JavaScript.
 */
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

/**
 * NEAR into yoctoNEAR, exactly. 24 decimals, no float.
 *
 * The same number a function-call key's allowance is denominated in, which is why it lives here
 * rather than in the tenancy module: an allowance and a price are the same unit.
 */
export function nearToYocto(near: string): bigint {
  return parseUnits(near, 24);
}

/**
 * NEP-145 `storage_balance_bounds().min` on a site: 0.007 NEAR.
 *
 * A mint into a fresh account has to fund at least this much, and a native buy attaches it as a
 * top-up that the site routes into its own generated `storage_deposit` before minting. Only a first
 * purchase into an unfunded wallet needs it; the contract refunds what it does not use to the
 * payer's pull ledger.
 */
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

/**
 * The asset a site's prices are denominated in.
 *
 * For a token site this is one `ft_metadata` call — the only way to know a token's decimals, and
 * the reason `min_slot_floor` is supplied at deploy time rather than discovered: `new` cannot await
 * an asynchronous read. For native settlement it is the constant, with no round trip.
 */
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
