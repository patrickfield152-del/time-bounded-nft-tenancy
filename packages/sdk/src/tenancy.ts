/**
 * Time-bounded tenancy (`rent(token_id, duration_secs, expected_rate_per_day)`, `end_rental`): 1. rent; 2. generate ed25519 key (`receiver_id = the site`, `method_names = ["edit"]`, `0.25 NEAR`); 3. `edit` via `submit` (NEP-366 safe, non-payable, contract checks `predecessor == tenant` and expiry); 4. delete key (1, 2, 3, 4).
 */
import { Account, KeyPair } from 'near-api-js';
import type { AccountId } from './near';
import { nearToYocto } from './settlement';

/** The only method a tenant's session key may call. Content writes, nothing financial. */
export const SESSION_METHODS: readonly string[] = ['edit'];

/** The allowance a session key is granted, in NEAR. */
export const SESSION_ALLOWANCE_NEAR = '0.25';

/** Gas allowance in yoctoNEAR (`0.25 NEAR`); unused yocto returns on key deletion. */
export const SESSION_ALLOWANCE_YOCTO: bigint = nearToYocto(SESSION_ALLOWANCE_NEAR);

/** A key pair plus its curve-prefixed public key, which is what the chain stores. */
export interface SessionKey {
  /** `ed25519:…` — what `addFunctionCallAccessKey` and `deleteKey` take. */
  publicKey: string;
  keyPair: KeyPair;
}

/** Generates an ed25519 `SessionKey` client-side without persisting the private key. */
export function newSessionKey(): SessionKey {
  const keyPair = KeyPair.fromRandom('ed25519');
  return { publicKey: keyPair.getPublicKey().toString(), keyPair };
}

export interface IssueSessionKeyOptions {
  /** The site account. This is `receiver_id`, and the only contract the key may call. */
  contractId: AccountId;
  /** Defaults to `SESSION_METHODS` — `["edit"]`. */
  methodNames?: readonly string[];
  /** Defaults to `SESSION_ALLOWANCE_YOCTO` (0.25 NEAR). */
  allowance?: bigint;
}

/** Adds an `edit`-only function-call key via `AddKey` for `tenant`; sign with `sessionAccount` and delete with `revokeSessionKey`. */
export async function issueSessionKey(
  tenant: Account,
  options: IssueSessionKeyOptions,
): Promise<SessionKey> {
  const session = newSessionKey();
  await tenant.addFunctionCallAccessKey({
    publicKey: session.publicKey,
    contractId: options.contractId,
    methodNames: [...(options.methodNames ?? SESSION_METHODS)],
    allowance: options.allowance ?? SESSION_ALLOWANCE_YOCTO,
  });
  return session;
}

/** Deletes a session key via `deleteKey`; `edit` rejects calls past `tenant_expires` even if the key remains. */
export async function revokeSessionKey(tenant: Account, publicKey: string): Promise<void> {
  await tenant.deleteKey(publicKey);
}

/** Builds a tenant signing `Account` backed by `session`. */
export function sessionAccount(tenantId: AccountId, session: SessionKey, rpcUrl: string): Account {
  return new Account(tenantId, rpcUrl, session.keyPair.toString());
}
