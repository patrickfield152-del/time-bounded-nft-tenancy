'use client';

/**
 * `useBuy` — the buy flow as state, with no UI attached.
 * Headless on purpose. `<BuyDialog>` is a thin default on top of this, and every real site will
 * restyle or replace it; the part that must not be reimplemented is the ordering below, because
 * getting it wrong is how a buyer signs terms they were never shown.
 * The ordering, and why each step is where it is:
 *   1. `readBuyContext` — quote, `encumbrance_hash` and the CHAIN's clock, all at one pinned block.
 *      Not three reads: a torn read across blocks means the buyer confirms a price from block N
 *      against terms from block N+1, and the anti-front-run guard is then guarding a state that
 *      never existed.
 *   2. Show the buyer that exact context and wait.
 *   3. Build from the SAME context that was displayed. Never re-read at submit time — a fresh read
 *      would pick up the very change `expected_terms` exists to reject, which is the one way to
 *      turn the guard into decoration.
 * **Signing is `near-connect-hooks`.** `submit` hands the built request to `useNearWallet()`'s
 * `callFunction`, which opens the user's wallet on exactly the call this quote produced. A site
 * that signs some other way passes its own handler to `buildRequest`'s output instead — the quote
 * and the request still come from here, so the numbers cannot drift from what was approved.
 */
import { useCallback, useState } from 'react';
import {
  buildBuyFrom,
  computeBuyBreakdown,
  computeElapsedWeeks,
  economicsFromTerms,
  readBuyContext,
  readSettlementAsset,
  readSiteTerms,
  toWalletCall,
  type AccountId,
  type BuildBuyOptions,
  type BuyContext,
  type CallRequest,
  type SettlementAsset,
} from '@stele/sdk';
import { useNearWallet } from 'near-connect-hooks';

import { useStele } from './context';

export type BuyPhase = 'idle' | 'quoting' | 'ready' | 'submitting' | 'confirmed' | 'error';

export interface BuyQuote {
  context: BuyContext;
  /** What this buyer pays: the effective floor on a claim, the take price on a take. */
  charged: bigint;
  /** What the displaced owner receives. `0n` on a claim — there is nobody to pay. */
  payout: bigint;
  /** True when nobody owns the slot yet, which is the cheaper branch. */
  isClaim: boolean;
  /** `null` for native NEAR. Carried so `buildRequest` attaches the right deposit. */
  settlementFt: AccountId | null;
  /**
   * What every amount in this quote is counted in — `NEAR`/24 natively, the token's own symbol and
   * decimals otherwise. Read with the quote, so a dialog never formats a 6-decimal price as 24.
   */
  currency: SettlementAsset;
  /** Escrowed rent this buyer would INHERIT. */
  unaccruedRent: bigint;
  /** `charged - unaccruedRent`, floored at zero. The number to lead with. */
  netCost: bigint;
  /** True when the inherited rent stream exceeds the purchase price outright. */
  isFreeCarry: boolean;
}

export interface UseBuyResult {
  phase: BuyPhase;
  quote: BuyQuote | null;
  error: Error | null;
  /** Read the context and compute the breakdown. Call this when the dialog opens. */
  prepare: () => Promise<BuyQuote | null>;
  /**
   * Build the request for the quote already shown. Returns `null` if `prepare` has not run — the
   * alternative, silently quoting inside `submit`, is exactly the re-read this flow prevents.
   */
  buildRequest: (
    options?: Pick<
      BuildBuyOptions,
      'maxPrice' | 'slippageBps' | 'deadlineNs' | 'deadlineSecs' | 'recipient' | 'storageTopUp'
    >,
  ) => CallRequest | null;
  /** Hand the built request to the connected wallet, then refetch the page. */
  submit: (request: CallRequest) => Promise<void>;
  reset: () => void;
  /** The connected account, or `''`. Empty means `submit` will refuse. */
  signedAccountId: string;
}

export function useBuy(key: string): UseBuyResult {
  const { config, reader, refetch } = useStele();
  const { callFunction, signedAccountId } = useNearWallet();
  const [phase, setPhase] = useState<BuyPhase>('idle');
  const [quote, setQuote] = useState<BuyQuote | null>(null);
  const [error, setError] = useState<Error | null>(null);

  const prepare = useCallback(async () => {
    setPhase('quoting');
    setError(null);
    try {
      const context = await readBuyContext(reader, config.site, key);
      if (!context.slot.registered) {
        throw new Error(
          `"${key}" is not registered on this site. Slots are closed by default — the site owner ` +
            'has to register it before it can be bought.',
        );
      }

      // Pinned to the SAME block as the quote, even though terms are cacheable: a cached value
      // belonging to a different site the app also renders is a real bug, and one call is cheaper
      // than the invalidation logic that avoids it.
      const terms = await readSiteTerms(reader, config.site, { blockId: context.blockHeight });
      const currency = await readSettlementAsset(reader, terms.settlementFt);

      // Guarded rather than passed straight in: a lapsed-but-uncleared term can leave the stored
      // purchase time a hair ahead of a rounded block timestamp, and `computeElapsedWeeks` throws
      // where the contract would simply compute zero weeks.
      const nowSecs = context.nowNs / 1_000_000_000n;
      const lastPurchaseSecs = context.slot.lastPurchaseNs / 1_000_000_000n;
      const elapsedWeeks =
        nowSecs < lastPurchaseSecs ? 0n : computeElapsedWeeks(nowSecs, lastPurchaseSecs);

      const breakdown = computeBuyBreakdown(
        context.slot.lastPrice,
        context.slot.floor,
        elapsedWeeks,
        economicsFromTerms(terms),
        context.slot.isUnclaimed,
      );

      // The site's own `quote` already returned the charged amount. Recomputing it locally and
      // asserting they agree is what catches a client/contract price divergence AT QUOTE TIME
      // rather than as a revert the buyer cannot interpret.
      if (breakdown.charged !== context.slot.charged) {
        throw new Error(
          `stele: price divergence on "${key}" — the contract quotes ${context.slot.charged} and the ` +
            `SDK computes ${breakdown.charged}. Refusing to build a transaction against either.`,
        );
      }

      const next: BuyQuote = {
        context,
        charged: context.slot.charged,
        payout: breakdown.payout,
        isClaim: context.slot.isUnclaimed,
        settlementFt: terms.settlementFt,
        currency,
        unaccruedRent: context.slot.unaccruedRent,
        netCost: context.slot.netCost,
        isFreeCarry: context.slot.isFreeCarry,
      };
      setQuote(next);
      setPhase('ready');
      return next;
    } catch (cause) {
      setError(cause instanceof Error ? cause : new Error(String(cause)));
      setPhase('error');
      return null;
    }
  }, [reader, config.site, key]);

  const buildRequest = useCallback<UseBuyResult['buildRequest']>(
    (options) => (quote ? buildBuyFrom(config.site, quote.context, quote.settlementFt, options) : null),
    [config.site, quote],
  );

  const submit = useCallback(
    async (request: CallRequest) => {
      if (!signedAccountId) {
        setError(
          new Error('@stele/react: connect a NEAR wallet before buying — useNearWallet() has no account'),
        );
        setPhase('error');
        return;
      }
      setPhase('submitting');
      setError(null);
      try {
        await callFunction(toWalletCall(request));
        setPhase('confirmed');
        await refetch();
      } catch (cause) {
        // Reported through `error`/`phase` rather than thrown: this is a UI hook, and a rejected
        // promise from a click handler is an unhandled rejection with the same message nobody sees.
        setError(cause instanceof Error ? cause : new Error(String(cause)));
        setPhase('error');
      }
    },
    [callFunction, signedAccountId, refetch],
  );

  const reset = useCallback(() => {
    setPhase('idle');
    setQuote(null);
    setError(null);
    void refetch();
  }, [refetch]);

  return { phase, quote, error, prepare, buildRequest, submit, reset, signedAccountId };
}
