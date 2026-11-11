'use client';

/**
 * `<SlotProvider>` — one chain read for the whole page, shared by every `<Slot>` under it.
 *
 * A page with twelve slots is ONE wave of parallel views, not twelve components each fetching their
 * own slot: the provider owns the read, so twelve slots cost one `readSlots` call. That is
 * load-bearing rather than an optimization — twelve independent fetches reintroduce exactly the
 * per-component round-trip pattern this design exists without.
 *
 * Live updates are an optional upgrade, not a default. `refetchOnFocus` is on for the cheapest
 * liveness that matters; `pollMs` is off because a page left open in a background tab should not
 * bill someone's RPC quota all afternoon.
 *
 * **Wallets are a separate layer.** This provider reads only, with `readerFromUrl(rpcUrl)`; sign-in
 * and signing belong to `near-connect-hooks`' `<NearProvider>` around the app, which `useBuy` uses
 * through `useNearWallet()`. Keeping the two apart is what lets a read-only page render with no
 * wallet at all.
 *
 *     <NearProvider config={{ network: 'testnet' }}>       // near-connect-hooks
 *       <SlotProvider config={site} rpcUrl={rpcUrl}>       // this file
 *         <Slot id="hero.headline" />
 *         <BuyDialog slotId="hero.headline" … />
 *       </SlotProvider>
 *     </NearProvider>
 */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import {
  contentUrl,
  readSlots,
  readerFromUrl,
  type ContentHash,
  type Reader,
  type SiteConfig,
  type SlotState,
} from '@stele/sdk';

import { ContentStore } from './content-store';

export interface SteleContextValue {
  config: SiteConfig;
  /** The SDK reader every hook and component reads through. */
  reader: Reader;
  slots: Record<string, SlotState>;
  contentStore: ContentStore;
  /** True only during the FIRST load. A background refetch must not blank a rendered page. */
  isLoading: boolean;
  error: Error | null;
  refetch: () => Promise<void>;
}

const SteleContext = createContext<SteleContextValue | null>(null);

export interface SlotProviderProps {
  config: SiteConfig;
  /** The JSON-RPC endpoint. A string, because it has to cross the server/client boundary. */
  rpcUrl: string;
  /**
   * Slot state read on the server, so the first paint has real prices and owners instead of a
   * skeleton. Pass the result of `readSlots` from a Server Component.
   */
  initialSlots?: SlotState[];
  /**
   * Content bytes resolved on the server, keyed by content hash. Re-verified on arrival — they
   * crossed a serialization boundary, and nothing crosses that boundary unchecked.
   */
  initialContent?: Record<ContentHash, Uint8Array>;
  /** Refetch when the tab regains focus. */
  refetchOnFocus?: boolean;
  /** Poll interval in ms. Off by default. */
  pollMs?: number;
  children: ReactNode;
}

export function SlotProvider({
  config,
  rpcUrl,
  initialSlots,
  initialContent,
  refetchOnFocus = true,
  pollMs,
  children,
}: SlotProviderProps) {
  const reader = useMemo(() => readerFromUrl(rpcUrl), [rpcUrl]);

  const [slots, setSlots] = useState<Record<string, SlotState>>(() =>
    Object.fromEntries((initialSlots ?? []).map((slot) => [slot.key, slot])),
  );
  const [isLoading, setIsLoading] = useState(initialSlots === undefined);
  const [error, setError] = useState<Error | null>(null);

  const contentStore = useMemo(() => {
    const store = new ContentStore({ contentUrl: (cid) => contentUrl(config.gateway, cid) });
    for (const [hash, bytes] of Object.entries(initialContent ?? {})) {
      store.hydrate(hash, bytes);
    }
    return store;
    // Intentionally keyed on the resolver alone: `initialContent` is a first-paint seed, and
    // rebuilding the store on every render would throw away the cache and refetch the page.
  }, [config.gateway]);

  // A refetch that resolves after a newer one must not overwrite it. Focus and poll can overlap
  // trivially — alt-tab twice while a slow RPC is in flight — and out-of-order responses would show
  // a stale owner on a page that had already updated.
  const generation = useRef(0);

  const refetch = useCallback(async () => {
    const mine = ++generation.current;
    try {
      const next = await readSlots(reader, config.site, config.keys);
      if (mine !== generation.current) return;
      setSlots(Object.fromEntries(next.map((slot) => [slot.key, slot])));
      setError(null);
    } catch (cause) {
      if (mine !== generation.current) return;
      setError(cause instanceof Error ? cause : new Error(String(cause)));
    } finally {
      if (mine === generation.current) setIsLoading(false);
    }
  }, [reader, config.site, config.keys]);

  useEffect(() => {
    if (initialSlots !== undefined) return;
    // Server-seeded pages skip the mount fetch entirely; focus and poll still apply.
    void refetch();
  }, [initialSlots, refetch]);

  useEffect(() => {
    if (!refetchOnFocus || typeof window === 'undefined') return;
    const onFocus = () => void refetch();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [refetchOnFocus, refetch]);

  useEffect(() => {
    if (!pollMs) return;
    const timer = setInterval(() => void refetch(), pollMs);
    return () => clearInterval(timer);
  }, [pollMs, refetch]);

  const value = useMemo<SteleContextValue>(
    () => ({ config, reader, slots, contentStore, isLoading, error, refetch }),
    [config, reader, slots, contentStore, isLoading, error, refetch],
  );

  return <SteleContext.Provider value={value}>{children}</SteleContext.Provider>;
}

export function useStele(): SteleContextValue {
  const value = useContext(SteleContext);
  if (!value) {
    throw new Error(
      '@stele/react: no <SlotProvider> above this component. Every <Slot> and every hook needs ' +
        'one, because the provider is what makes a whole page a single chain read.',
    );
  }
  return value;
}
