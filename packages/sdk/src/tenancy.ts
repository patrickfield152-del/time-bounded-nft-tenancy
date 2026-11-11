/**
 * Time-bounded tenancy: the session key a tenant uses to write content without a wallet prompt.
 *
 * The mechanism, end to end:
 *
 *   1. A tenant calls `rent(token_id, duration_secs, expected_rate_per_day)` and pays for the term.
 *   2. They generate a fresh key pair HERE and add it to their own account as a **function-call
 *      key** restricted to `receiver_id = the site` and `method_names = ["edit"]`, funded with a
 *      gas-only allowance.
 *   3. Every `edit` in the term is signed by that key: no extension, no prompt, no wallet in the
 *      loop. The site still checks `predecessor == tenant` and
 *      `block_timestamp() < tenant_expires` on every call, because the key cannot enforce either.
 *   4. At `end_rental` — or whenever the tenant wants out — the key is deleted.
 *
 * **A function-call key has no expiry field.** It has a receiver, a method list and an allowance,
 * and none of them is time. The term is enforced by the contract; that is the only authority, and
 * it is also why NEP-366 meta-transactions are safe here — a relayer does not check the allowance,
 * so a design that relied on the key running dry would be relying on billing rather than on time.
 *
 * **0.25 NEAR is the allowance**, matching the default wallet sign-in grants and sized for a term
 * of ordinary editing. An allowance is a gas budget, not a balance the tenant loses: unused yocto
 * returns to the account when the key is deleted. When it runs out the key stops working — it does
 * not fail a call an owner-key would have allowed, because `edit` is the only method it may call.
 *
 * **Deletion costs a transaction**, signed by a full-access key (the wallet), and it cannot attach
 * a deposit — which is also why `edit` itself must be non-payable. A tenant who never deletes the
 * key has not granted anyone else anything: the key is in their own account and the site's term
 * check has already expired.
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
 *
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
 *
 * @param tenant A signing account for the tenant — the wallet, or an `Account` built from their
 *   key. The transaction is an ordinary `AddKey` and costs the tenant gas.
 *
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
 *
 * `deleteKey` needs a full-access signature, so this is the wallet's transaction, not the session
 * key's. Call it when the term ends (or is ended early); a forgotten key cannot outlive the term,
 * because the site's `edit` rejects any call past `tenant_expires`.
 */
export async function revokeSessionKey(tenant: Account, publicKey: string): Promise<void> {
  await tenant.deleteKey(publicKey);
}

/**
 * An `Account` that signs as the tenant with the session key — the shape `submit` takes.
 *
 * Use it for `edit` only: the key's permission is the site's `edit` method, so anything else is
 * rejected by the protocol before the contract runs.
 */
export function sessionAccount(tenantId: AccountId, session: SessionKey, rpcUrl: string): Account {
  return new Account(tenantId, rpcUrl, session.keyPair);
}
