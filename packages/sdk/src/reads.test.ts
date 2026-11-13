//! SDK-side parity for `slot_bundle` and the locally computed encumbrance.
//!
//! The fixture is the same file the Rust integration test reads, so a change to
//! the preimage layout fails on whichever side drifted first.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  encumbranceHash,
  readBuyContext,
  readSlots,
  type RentalRecord,
  type SlotBundleView,
  type SlotRecord,
} from './reads';
import type { Reader } from './near';

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
