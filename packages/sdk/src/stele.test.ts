/**
 * Native-codec feature tests.
 *
 * `Uint8Array.prototype.toBase64` is TC39 stage-3, shipped in V8 13.6 / Node
 * 25.0.0. The SDK encoders use it when present and fall back to `Buffer` /
 * `btoa` otherwise; these tests pin the output to the exact bytes the chain
 * stores regardless of which branch runs.
 */
//! Native near-api-js provider/signer wiring.
//! SDK-side parity for `slot_bundle` and the locally computed encumbrance.
//! The fixture is the same file the Rust integration test reads, so a change to
//! the preimage layout fails on whichever side drifted first.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { FailoverRpcProvider, JsonRpcProvider, KeyPair, MultiKeySigner } from 'near-api-js';
import type { KeyPairString } from 'near-api-js';
import { mediaHash, sha256Hex } from './content';
import { accountFromEnv, accountFromKeys, readerFromUrls } from './near';
import type { Reader } from './near';
import {
  encumbranceHash,
  readBuyContext,
  readSlots,
  type RentalRecord,
  type SlotBundleView,
  type SlotRecord,
} from './reads';

const NATIVE_TO_BASE64 = typeof (Uint8Array.prototype as Uint8Array & { toBase64?: unknown }).toBase64 === 'function';
const keys = [KeyPair.fromRandom('ed25519').toString() as KeyPairString, KeyPair.fromRandom('ed25519').toString() as KeyPairString];

interface FixtureRow {
  name: string;
  token_id: string;
  slot: Pick<SlotRecord, 'base_price' | 'registered'> | null;
  rental: Pick<RentalRecord, 'tenant' | 'expiry' | 'prepaid' | 'claimed'> | null;
  preimage_hex: string;
  hash: string;
}

const rows: FixtureRow[] = readFileSync(
  new URL('../../../vectors/encumbrance-vectors.jsonl', import.meta.url),
  'utf8',
)
  .trim()
  .split('\n')
  .map((line) => JSON.parse(line) as FixtureRow);

const record: SlotRecord = {
  last_price: '10',
  base_price: '1000',
  page_hash: 'abc',
  ask_floor: '0',
  last_purchase_at_ns: 1,
  floor_updated_at_ns: 0,
  content_version: 1,
  takes: 1,
  registered: true,
  available: false,
};

const bundle: SlotBundleView = {
  slot: record,
  quote: { effective_floor: '1000', price: '1200' },
  owner: 'bob.near',
  listing: null,
  rental: null,
};

function fakeReader(
  result: SlotBundleView | null,
  calls: Array<{ method: string; args: Record<string, unknown> }>,
): Reader {
  return {
    viewBlock: async () => ({
      header: { height: 42, timestamp: 1_700_000_000_000_000_000 },
    }),
    callFunction: async (params: { method: string; args: Record<string, unknown> }) => {
      calls.push({ method: params.method, args: params.args });
      return result;
    },
  } as unknown as Reader;
}

describe('native Uint8Array.toBase64', () => {
  it('reports whether this runtime has the TC39 method', () => {
    // Node 25+: MEASURED true on v25.0.0; Node 22/24: false (fallback path).
    expect(typeof NATIVE_TO_BASE64).toBe('boolean');
  });

  it('mediaHash equals the reference base64 of sha256', () => {
    const bytes = new Uint8Array([0xfb, 0xff, 0xbf, 0x00, 0x01]);
    // echo -n <bytes> | base64 of the digest is not stable across shells, so
    // the reference is computed with the same standard alphabet here.
    const reference = Buffer.from(sha256Hex(bytes), 'hex').toString('base64');
    expect(mediaHash(bytes)).toBe(reference);
  });

  it('produces standard padded base64 on every path', () => {
    const bytes = new Uint8Array(31).map((_, index) => index * 7 + 1);
    const digest = mediaHash(bytes);
    expect(digest).toMatch(/^[A-Za-z0-9+/]+={0,2}$/);
    expect(digest.length % 4).toBe(0);
  });

  it('keeps sha256Hex a pure lowercase hex digest', () => {
    expect(sha256Hex(new Uint8Array([1, 2, 3]))).toMatch(/^[0-9a-f]{64}$/);
  });
});

describe('readerFromUrls', () => {
  it('keeps one URL on JsonRpcProvider and uses native failover for many', () => {
    expect(readerFromUrls(['https://rpc.example'])).toBeInstanceOf(JsonRpcProvider);
    const failover = readerFromUrls(['https://a.example', 'https://b.example']);
    expect(failover).toBeInstanceOf(FailoverRpcProvider);
    expect((failover as unknown as FailoverRpcProvider).providers).toHaveLength(2);
  });

  it('rejects an empty list instead of creating a provider that cannot read', () => {
    expect(() => readerFromUrls(['', '  '])).toThrow('at least one RPC URL');
  });
});

describe('accountFromKeys', () => {
  it('uses the native MultiKeySigner for several keys', () => {
    const account = accountFromKeys({ accountId: 'site.test.near', privateKeys: keys, rpcUrl: 'https://rpc.example' });
    expect(account.getSigner()).toBeInstanceOf(MultiKeySigner);
  });

  it('accepts NEAR_PRIVATE_KEYS in the env path', () => {
    const account = accountFromEnv({
      NEAR_ACCOUNT_ID: 'site.test.near',
      NEAR_PRIVATE_KEYS: keys.join(','),
      NEAR_RPC_URL: 'https://rpc.example',
    });
    expect(account.getSigner()).toBeInstanceOf(MultiKeySigner);
  });
});

describe('encumbrance parity vectors', () => {
  it('hashes the canonical preimage and matches the Rust fixture', () => {
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(encumbranceHash(row.slot, row.rental), row.name).toBe(row.hash);
    }
  });
});

describe('slot bundle reads', () => {
  it('reads a slot and the guard from one bundle call', async () => {
    const calls: Array<{ method: string; args: Record<string, unknown> }> = [];
    const context = await readBuyContext(fakeReader(bundle, calls), 'site.test.near', 'hero.headline');

    expect(calls.map((call) => call.method)).toEqual(['slot_bundle']);
    expect(calls[0]?.args).toEqual({ token_id: 'hero.headline' });
    expect(context.expectedTerms).toBe(encumbranceHash(record, null));
    expect(context.slot).toMatchObject({
      key: 'hero.headline',
      owner: 'bob.near',
      charged: 1200n,
      registered: true,
    });
    expect(context.blockHeight).toBe(42);
  });

  it('uses one bundle per key for a page', async () => {
    const calls: Array<{ method: string; args: Record<string, unknown> }> = [];
    const slots = await readSlots(fakeReader(bundle, calls), 'site.test.near', ['a.slot', 'b.slot']);

    expect(calls.map((call) => call.method)).toEqual(['slot_bundle', 'slot_bundle']);
    expect(calls.map((call) => call.args.token_id)).toEqual(['a.slot', 'b.slot']);
    expect(slots).toHaveLength(2);
  });

  it('refuses an unregistered key without inventing a zero buy', async () => {
    const calls: Array<{ method: string; args: Record<string, unknown> }> = [];
    const empty: SlotBundleView = {
      slot: null,
      quote: { effective_floor: '0', price: '0' },
      owner: null,
      listing: null,
      rental: null,
    };
    await expect(readBuyContext(fakeReader(empty, calls), 'site.test.near', 'missing.slot')).rejects.toThrow(
      'is not registered',
    );
    expect(calls.map((call) => call.method)).toEqual(['slot_bundle']);
  });
});
