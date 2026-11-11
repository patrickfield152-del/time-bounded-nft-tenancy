import { formatUnits, type SettlementAsset } from '@stele/sdk';

/**
 * An amount in a board's own currency, as a person reads it: `2.5 USDG`, `0.0014 NEAR`.
 *
 * The decimals come from the quote and never from a default, because the same integer is two
 * dollars on a 6-decimal board and two trillionths of a NEAR on a 24-decimal one.
 */
export function formatAmount(amount: bigint, currency: SettlementAsset): string {
  return `${formatUnits(amount, currency.decimals)} ${currency.symbol}`;
}
