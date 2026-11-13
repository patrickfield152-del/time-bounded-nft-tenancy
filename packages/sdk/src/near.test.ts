//! Native near-api-js provider/signer wiring.

import { FailoverRpcProvider, JsonRpcProvider, KeyPair, MultiKeySigner } from 'near-api-js';
import type { KeyPairString } from 'near-api-js';
import { describe, expect, it } from 'vitest';
import { accountFromEnv, accountFromKeys, readerFromUrls } from './near';

const keys = [KeyPair.fromRandom('ed25519').toString() as KeyPairString, KeyPair.fromRandom('ed25519').toString() as KeyPairString];

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
