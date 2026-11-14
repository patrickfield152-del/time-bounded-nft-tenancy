'use client';

/**
 * `<Slot>` — the component a builder actually writes.
 *     <Slot id="hero.headline" as="h1" className="text-6xl" fallback="Ship faster." />
 * **`fallback` is the most important prop in this package.** It is what renders when a slot is
 * unclaimed, and it is what makes the page look finished on day one — the difference between a
 * demo and a dead grid of empty boxes. It is also the universal failure mode: unclaimed, never
 * edited, gateway down, hash mismatch, unknown scheme version. Every one of those paints
 * `fallback`, so a page whose storage has gone entirely dark still looks like the product it is
 * pretending to be.
 * **Import it as `Slot` from `@stele/react`, never as a bare default.** `slot` is crowded frontend
 * vocabulary — Web Components have a native `<slot>` element and React has an established "slots"
 * composition pattern — and a named import is how a reader tells at a glance which `Slot` is meant.
 */
import { createElement, useEffect, useState, type ElementType, type ReactNode } from 'react';
import { ContentKind } from '@stele/sdk';

import { useSlot, type UseSlotResult } from './useSlot';

const decoder = new TextDecoder();

/** Client payload shapes (`text`, `link` `{ "href": string, "label": string }`, `image`, `video` `{ "src": string }`; custom sites use `useSlot()`). */
export interface SlotProps {
  /** Slot key (`hero.headline`), emitted as `data-slot` (never DOM `id`). */
  id: string;
  /** The element to render. Defaults per kind: `span` for text, `a` for link, `img` for image. */
  as?: ElementType;
  className?: string;
  /** Rendered when verified content is absent (`src` string for `image`/`video`). */
  fallback?: ReactNode;
  /** Escape hatch: render the slot yourself from the resolved state. */
  children?: (slot: UseSlotResult) => ReactNode;
}

export function Slot({ id, as, className, fallback, children }: SlotProps) {
  const slot = useSlot(id);
  if (children) return <>{children(slot)}</>;

  const kind = slot.content.state === 'ready' ? slot.content.kind : slot.definition.contentKind;

  if (kind === ContentKind.Image || kind === ContentKind.Video) {
    return <MediaSlot slot={slot} id={id} as={as} className={className} fallback={fallback} />;
  }

  if (slot.showFallback) {
    return createElement(as ?? (kind === ContentKind.Link ? 'a' : 'span'), { className, 'data-slot': id }, fallback);
  }

  if (kind === ContentKind.Link) {
    const link = parseJson<{ href?: string; label?: string }>(slot.content);
    // A link whose payload does not parse, or names no destination, is a failure like any other —
    // rendering an anchor with no href is a dead element that looks deliberate.
    if (!link?.href) return createElement(as ?? 'a', { className, 'data-slot': id }, fallback);
    return createElement(as ?? 'a', { className, href: link.href, 'data-slot': id }, link.label ?? link.href);
  }

  return createElement(as ?? 'span', { className, 'data-slot': id }, slot.content.state === 'ready' ? decoder.decode(slot.content.payload) : '');
}

/** Renders verified bytes via blob URL rather than `<img src={gatewayUrl}>` (8-byte check). */
function MediaSlot({
  slot,
  id,
  as,
  className,
  fallback,
}: {
  slot: UseSlotResult;
  id: string;
  as?: ElementType;
  className?: string;
  fallback?: ReactNode;
}) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const content = slot.content;

  useEffect(() => {
    if (content.state !== 'ready') {
      setObjectUrl(null);
      return;
    }
    // Runtimes without createObjectURL (some test environments) fall back instead of throwing on
    // every image slot.
    if (typeof URL === 'undefined' || typeof URL.createObjectURL !== 'function') return;

    const url = URL.createObjectURL(new Blob([content.payload as BlobPart]));
    setObjectUrl(url);
    // Revoked on unmount and on every content change, or a page that refetches through a few takes
    // leaks a blob per image for the session.
    return () => URL.revokeObjectURL(url);
  }, [content]);

  const src = objectUrl ?? (typeof fallback === 'string' ? fallback : undefined);
  if (!src) {
    return createElement(
      as ?? 'span',
      { className, 'data-slot': id },
      typeof fallback === 'string' ? null : fallback,
    );
  }

  const element = as ?? (slot.definition.kind === 'video' ? 'video' : 'img');
  return createElement(element, { className, src, 'data-slot': id, alt: element === 'img' ? '' : undefined });
}

function parseJson<T>(content: UseSlotResult['content']): T | null {
  if (content.state !== 'ready') return null;
  try {
    return JSON.parse(decoder.decode(content.payload)) as T;
  } catch {
    return null;
  }
}
