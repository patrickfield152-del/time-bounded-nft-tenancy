'use client';

import { useEffect, useSyncExternalStore } from 'react';
import type { SlotDefinition, SlotState } from '@stele/sdk';

import { useStele } from './context';
import type { ContentStatus } from './store';

export interface UseSlotResult {
  /** The config entry — kind and configured floor. */
  definition: SlotDefinition;
  /** On-chain state, or `undefined` until the first read lands. */
  state: SlotState | undefined;
  /** Verified content, or why there is none. */
  content: ContentStatus;
  /** True whenever no verified content is ready (renders `fallback`). */
  showFallback: boolean;
  isLoading: boolean;
}

/** Returns slot state and verified content from `<SlotProvider>`. */
export function useSlot(key: string): UseSlotResult {
  const { config, slots, contentStore, isLoading } = useStele();

  const definition = config.slotsByKey[key];
  if (!definition) {
    throw new Error(
      `@stele/react: "${key}" is not in the site config. Slot keys are on-chain token ids — add it ` +
        'to the site config and register it, rather than passing a key that resolves to nothing.',
    );
  }

  const state = slots[key];
  const contentHash = state?.contentHash ?? null;

  const content = useSyncExternalStore(
    (onChange) => contentStore.subscribe(onChange),
    () => (contentHash ? contentStore.get(contentHash) : IDLE),
    // Server snapshot. `useSyncExternalStore` requires this to be referentially stable across
    // calls or React throws an infinite-loop error, which is why IDLE is a module constant rather
    // than an object literal.
    () => (contentHash ? contentStore.get(contentHash) : IDLE),
  );

  useEffect(() => {
    if (contentHash) void contentStore.load(contentHash);
  }, [contentHash, contentStore]);

  return {
    definition,
    state,
    content,
    showFallback: content.state !== 'ready',
    isLoading,
  };
}

const IDLE: ContentStatus = { state: 'idle' };
