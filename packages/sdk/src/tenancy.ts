/**
 * Time-bounded tenancy: a session key lets a tenant write content without a
 * wallet prompt.
 * 1. The tenant calls `rent(token_id, duration_secs, expected_rate_per_day)`.
 * 2. A fresh key pair is generated here and added as a function-call key.
 *    It allows only `receiver_id = the site` and
 *    `method_names = ["edit"]`, with a gas-only allowance.
 * 3. Every `edit` in the term uses that key. The site still checks
 *    `predecessor == tenant` and the term on every call.
 * 4. At `end_rental` the key is deleted.
 * - A function-call key has no expiry field. The contract enforces the term.
 * - NEP-366 meta-transactions are safe here: a relayer does not check the
 *   allowance, so the key running dry is billing, not time.
 * - The allowance is 0.25 NEAR. It is a gas budget, not a balance. Unused
 *   yocto returns when the key is deleted.
 * - Deletion costs one transaction signed by a full-access key. `edit` must
 *   be non-payable because that transaction attaches no deposit.
 */
import { Account, KeyPair } from 'near-api-js';
import type { AccountId } from './near';
import { nearToYocto } from './settlement';

/** The only method a tenant's session key may call. Content writes, nothing financial. */
export const SESSION_METHODS: readonly string[] = ['edit'];

/** The allowance a session key is granted, in NEAR. */
export const SESSION_ALLOWANCE_NEAR = '0.25';

/**
 * The same allowance in yoctoNEAR, exactly. Gas budget, not a balance: unused yocto is returned to
 * the tenant's account when the key is deleted.
 */
export const SESSION_ALLOWANCE_YOCTO: bigint = nearToYocto(SESSION_ALLOWANCE_NEAR);

/** A key pair plus its curve-prefixed public key, which is what the chain stores. */
export interface SessionKey {
  /** `ed25519:…` — what `addFunctionCallAccessKey` and `deleteKey` take. */
  publicKey: string;
  keyPair: KeyPair;
}

/**
 * A fresh ed25519 key pair.
 * Generated client-side and never transmitted: the private half exists only in this object (and in
 * whatever the caller persists it in). Nothing in this package stores it.
 */
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

/**
 * Adds a session key to the tenant's account.
 * @param tenant A signing account for the tenant — the wallet, or an `Account` built from their
 *   key. The transaction is an ordinary `AddKey` and costs the tenant gas.
 * The returned `SessionKey` is the caller's to keep: sign edits with `sessionAccount`, and delete it
 * with `revokeSessionKey` at the end of the term.
 */
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

/**
 * Deletes a session key from the tenant's account.
 * `deleteKey` needs a full-access signature, so this is the wallet's transaction, not the session
 * key's. Call it when the term ends (or is ended early); a forgotten key cannot outlive the term,
 * because the site's `edit` rejects any call past `tenant_expires`.
 */
export async function revokeSessionKey(tenant: Account, publicKey: string): Promise<void> {
  await tenant.deleteKey(publicKey);
}

/**
 * An `Account` that signs as the tenant with the session key — the shape `submit` takes.
 * Use it for `edit` only: the key's permission is the site's `edit` method, so anything else is
 * rejected by the protocol before the contract runs.
 */
export function sessionAccount(tenantId: AccountId, session: SessionKey, rpcUrl: string): Account {
  return new Account(tenantId, rpcUrl, session.keyPair.toString());
}
