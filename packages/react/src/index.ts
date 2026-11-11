/**
 * `@stele/react` — render a stele board, read a slot's verified content, and run a buy flow.
 *
 * Setup, in one place:
 *
 *     <NearProvider config={{ network: 'testnet' }}>      // near-connect-hooks
 *       <SlotProvider config={site} rpcUrl={rpcUrl}>      // one read for the whole page
 *         <Slot id="hero.headline" fallback="Ship faster." />
 *         <BuyDialog slotId="hero.headline" open={open} onClose={close} />
 *       </SlotProvider>
 *     </NearProvider>
 *
 * `<SlotProvider>` is reads only and works without a wallet. `<NearProvider>` is required by
 * `useBuy`/`<BuyDialog>`, which submit through `useNearWallet()`'s `callFunction`; a site that signs
 * some other way passes `onConfirm` and keeps the quote from `useBuy`.
 */
export { SlotProvider, useStele } from './context';
export type { SlotProviderProps, SteleContextValue } from './context';

export { Slot } from './Slot';
export type { SlotProps } from './Slot';

export { useSlot } from './useSlot';
export type { UseSlotResult } from './useSlot';

export { BuyDialog } from './BuyDialog';
export type { BuyDialogProps } from './BuyDialog';

export { useBuy } from './useBuy';
export type { UseBuyResult, BuyQuote, BuyPhase } from './useBuy';

export { formatAmount } from './format';

export { ContentStore, fetchAndVerify } from './content-store';
export type { ContentStatus, FetchContentOptions } from './content-store';
