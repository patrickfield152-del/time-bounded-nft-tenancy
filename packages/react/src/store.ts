/**
 * Fetches content by hash and verifies sha256 before `<Slot>` renders; unverified bytes degrade to `fallback` (never `<img src={gatewayUrl}>`).
 */
import {
  contentHashToCid,
  readContent,
  type ContentFailure,
  type ContentHash,
  type ContentResult,
} from '@stele/sdk';

export type ContentStatus =
  | { state: 'idle' }
  | { state: 'loading' }
  | { state: 'ready'; kind: number; payload: Uint8Array }
  | { state: 'failed'; reason: ContentFailure | 'fetch-failed' };

/** `fetch-failed` = gateway error; `hash-mismatch` = content error. */
export interface FetchContentOptions {
  contentUrl: (cid: string) => string;
  signal?: AbortSignal;
  fetchImpl?: typeof fetch;
}

export async function fetchAndVerify(
  contentHash: ContentHash,
  options: FetchContentOptions,
): Promise<ContentStatus> {
  const doFetch = options.fetchImpl ?? globalThis.fetch;
  let bytes: Uint8Array;

  try {
    const response = await doFetch(options.contentUrl(contentHashToCid(contentHash)), {
      signal: options.signal,
    });
    if (!response.ok) return { state: 'failed', reason: 'fetch-failed' };
    bytes = new Uint8Array(await response.arrayBuffer());
  } catch {
    // Includes aborts. A caller that aborted is unmounting or refetching and discards this anyway;
    // treating it as a failure is harmless and keeps the branch count down.
    return { state: 'failed', reason: 'fetch-failed' };
  }

  const result: ContentResult = readContent(bytes, contentHash);
  if (!result.ok) return { state: 'failed', reason: result.reason };
  return { state: 'ready', kind: result.kind, payload: result.payload };
}

/** Immutable hash-keyed cache with in-flight request deduplication. */
export class ContentStore {
  private readonly entries = new Map<ContentHash, ContentStatus>();
  private readonly inFlight = new Map<ContentHash, Promise<ContentStatus>>();
  private readonly listeners = new Set<() => void>();

  constructor(private readonly options: FetchContentOptions) {}

  get(contentHash: ContentHash): ContentStatus {
    return this.entries.get(contentHash) ?? { state: 'idle' };
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Idempotent. Safe to call from a render effect on every commit. */
  load(contentHash: ContentHash): Promise<ContentStatus> {
    const existing = this.inFlight.get(contentHash);
    if (existing) return existing;

    const cached = this.entries.get(contentHash);
    if (cached && cached.state !== 'idle') return Promise.resolve(cached);

    this.entries.set(contentHash, { state: 'loading' });
    const request = fetchAndVerify(contentHash, this.options).then((status) => {
      this.entries.set(contentHash, status);
      this.inFlight.delete(contentHash);
      this.emit();
      return status;
    });

    this.inFlight.set(contentHash, request);
    // Emit the `loading` transition too, or a component that mounted in `idle` never re-renders
    // into its own pending state.
    this.emit();
    return request;
  }

  /**
   * Seeds content resolved on the server, so server-rendered text appears in the first paint instead
   * of flashing `fallback` and then filling in. The bytes are re-verified here rather than trusted:
   * they crossed a serialization boundary, and the point of the gate is that nothing crosses it
   * unchecked.
   */
  hydrate(contentHash: ContentHash, bytes: Uint8Array): void {
    const result = readContent(bytes, contentHash);
    this.entries.set(
      contentHash,
      result.ok
        ? { state: 'ready', kind: result.kind, payload: result.payload }
        : { state: 'failed', reason: result.reason },
    );
  }

  private emit(): void {
    for (const listener of this.listeners) listener();
  }
}
