'use client';

/** Default `<BuyDialog>` UI; numbers come directly from `useBuy`. NEAR `ft_transfer_call` uses one prompt (no separate approval). */
import { useEffect, type ReactNode } from 'react';
import { DEFAULT_SLIPPAGE_BPS, type CallRequest } from '@stele/sdk';

import { formatAmount } from './format';
import { useBuy, type BuyQuote } from './useBuy';

export interface BuyDialogProps {
  slotId: string;
  open: boolean;
  onClose: () => void;
  /** Custom submit handler; defaults to `useNearWallet()`. */
  onConfirm?: (request: CallRequest, quote: BuyQuote) => void | Promise<void>;
  /** Buy on someone else's behalf. Native settlement only — see `buildBuy`. */
  recipient?: string;
  /** Slippage headroom over the quoted price, in bps. */
  slippageBps?: bigint;
  children?: (state: ReturnType<typeof useBuy>) => ReactNode;
}

export function BuyDialog({
  slotId,
  open,
  onClose,
  onConfirm,
  recipient,
  slippageBps = DEFAULT_SLIPPAGE_BPS,
  children,
}: BuyDialogProps) {
  const buy = useBuy(slotId);
  const { phase, quote, error, prepare, buildRequest, submit, reset, signedAccountId } = buy;

  useEffect(() => {
    if (open && phase === 'idle') void prepare();
    if (!open && phase !== 'idle') reset();
  }, [open, phase, prepare, reset]);

  if (!open) return null;
  if (children) return <>{children(buy)}</>;

  return (
    <div role="dialog" aria-label={`Buy ${slotId}`}>
      {phase === 'quoting' && <p>Quoting…</p>}

      {phase === 'error' && (
        <>
          <p role="alert">{error?.message}</p>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </>
      )}

      {phase === 'confirmed' && (
        <>
          <p role="status">Confirmed. Finalizing slot ownership.</p>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </>
      )}

      {(phase === 'ready' || phase === 'submitting') && quote && (
        <>
          {/* Headline `netCost` subtracts inherited unaccrued rent from `charged`. */}
          {quote.unaccruedRent > 0n ? (
            <>
              <p>
                Take <code>{slotId}</code> for{' '}
                <strong>{formatAmount(quote.netCost, quote.currency)}</strong> net
              </p>
              <p>
                Pay {formatAmount(quote.charged, quote.currency)} now and inherit{' '}
                {formatAmount(quote.unaccruedRent, quote.currency)} escrowed rent over the remaining term.
                {quote.isFreeCarry
                  ? ' That is more than the purchase price: this position pays for itself.'
                  : ''}
              </p>
              <p>Rented now: the tenant controls content until the term ends.</p>
            </>
          ) : (
            <p>
              {quote.isClaim ? 'Claim' : 'Take'} <code>{slotId}</code> for{' '}
              <strong>{formatAmount(quote.charged, quote.currency)}</strong>
            </p>
          )}

          {/* Payout is `payout_bps` of current effective floor, not prior purchase price. */}
          {!quote.isClaim && (
            <p>
              Current owner receives {formatAmount(quote.payout, quote.currency)} (slot floor guaranteed; may be below prior price after reversion).
            </p>
          )}

          <p>Others may take this slot at the formula price.</p>

          {quote.settlementFt !== null && (
            <p>
              Wallet prompt: {quote.currency.symbol} transfer up to {formatAmount(quote.charged, quote.currency)} plus slippage margin.
            </p>
          )}

          {!signedAccountId && <p role="status">Connect wallet to confirm.</p>}

          <button
            type="button"
            disabled={phase === 'submitting' || !signedAccountId}
            onClick={() => {
              const request = buildRequest({ recipient, slippageBps });
              if (!request) return;
              if (onConfirm) {
                void onConfirm(request, quote);
                return;
              }
              void submit(request);
            }}
          >
            {phase === 'submitting' ? 'Submitting…' : 'Confirm'}
          </button>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
        </>
      )}
    </div>
  );
}
