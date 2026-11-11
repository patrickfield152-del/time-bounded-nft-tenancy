'use client';

/**
 * `<BuyDialog>` — the default confirm step.
 *
 * Deliberately unstyled and deliberately thin. Every real site will restyle or replace it, so the
 * value here is not the markup: it is that the numbers shown are the numbers `useBuy` builds the
 * transaction from, and that the two cannot drift. A site that writes its own dialog on `useBuy`
 * inherits that; a site that quotes separately from what it submits does not.
 *
 * **On NEAR there is no approval step.** The native path attaches the price to one call; the token
 * path is a single `ft_transfer_call` whose amount is the price. So the old "your wallet will ask
 * twice" warning is gone — a token purchase is one prompt, and saying otherwise would teach users
 * that a second prompt is expected.
 */
import { useEffect, type ReactNode } from 'react';
import { DEFAULT_SLIPPAGE_BPS, type CallRequest } from '@stele/sdk';

import { formatAmount } from './format';
import { useBuy, type BuyQuote } from './useBuy';

export interface BuyDialogProps {
  slotId: string;
  open: boolean;
  onClose: () => void;
  /**
   * Receives the request built from the quote the user was actually shown. Omit and the dialog
   * submits through `useNearWallet()` itself — the app-level wallet that `useBuy` already reads.
   */
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
      {phase === 'quoting' && <p>Reading the current price…</p>}

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
          <p role="status">Confirmed. The slot is yours once the transaction finalizes.</p>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </>
      )}

      {(phase === 'ready' || phase === 'submitting') && quote && (
        <>
          {/*
            On an ENCUMBERED position the headline number is `netCost`, not `charged`. The buyer
            inherits the unaccrued rent, so `charged` alone tells them they are paying full price
            for something they cannot use — when they are buying an asset that hands part of that
            back over the remaining term. `charged` is still shown, because it is what leaves their
            wallet and what the transaction is built from.
          */}
          {quote.unaccruedRent > 0n ? (
            <>
              <p>
                Take <code>{slotId}</code> for{' '}
                <strong>{formatAmount(quote.netCost, quote.currency)}</strong> net
              </p>
              <p>
                You pay {formatAmount(quote.charged, quote.currency)} now and inherit{' '}
                {formatAmount(quote.unaccruedRent, quote.currency)} of rent already escrowed on this
                slot, which streams to you over the rest of the tenancy.
                {quote.isFreeCarry
                  ? ' That is more than the purchase price: this position pays for itself.'
                  : ''}
              </p>
              <p>
                It is rented right now, so the tenant — not you — controls its content until the
                term ends.
              </p>
            </>
          ) : (
            <p>
              {quote.isClaim ? 'Claim' : 'Take'} <code>{slotId}</code> for{' '}
              <strong>{formatAmount(quote.charged, quote.currency)}</strong>
            </p>
          )}

          {/*
            What the payout guarantees, and what it does NOT. The payout is `payout_bps` of the
            CURRENT effective floor, not of what the displaced holder paid; reversion separates the
            two. The guarantee that survives every input is the floor one, so that is what this says.
          */}
          {!quote.isClaim && (
            <p>
              The current owner receives {formatAmount(quote.payout, quote.currency)} — never less
              than this slot&rsquo;s floor, which is guaranteed. Whether it beats what they paid is
              not: the payout tracks the slot&rsquo;s floor now, and a price that has reverted since
              they bought can leave them below it.
            </p>
          )}

          <p>Anyone can take this slot from you later at a formula price. That is the mechanic.</p>

          {quote.settlementFt !== null && (
            <p>
              Your wallet will ask once: a {quote.currency.symbol} transfer to the site for up to{' '}
              {formatAmount(quote.charged, quote.currency)} plus a small margin in case the price
              moves before your purchase lands.
            </p>
          )}

          {!signedAccountId && <p role="status">Connect a NEAR wallet to confirm.</p>}

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
