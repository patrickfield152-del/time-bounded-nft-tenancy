/**
 * Chain plumbing: one JSON-RPC reader, and an `Account` only where something has to sign.
 *
 * Two separations this file exists to hold.
 *
 * **A read never needs a key.** `readerFromEnv()`/`readerFromUrl()` return a plain
 * `near-api-js` provider, which is all the read path, the React provider and the demo use. The
 * provider is never constructed with an account attached, so a page cannot accidentally sign.
 *
 * **A write is a value, not a side effect.** `CallRequest` is the whole write surface: a contract,
 * a method, JSON args and an attached deposit in yoctoNEAR. Apps hand it to whatever wallet layer
 * they already have (`toWalletCall` reshapes it for `@hot-labs/near-connect`'s `callFunction`), and
 * scripts hand it to `submit` with an `Account` built from env. The SDK never picks a nonce, a gas
 * price or a submission moment.
 *
 * No address and no network id is baked in here: the RPC URL comes from the caller, so the same
 * build serves testnet, mainnet and a local sandbox.
 */
import {
  Account,
  JsonRpcProvider,
  actions,
  type BlockReference,
  type KeyPairString,
  type RpcTransactionResponse,
} from 'near-api-js';

/** A NEAR account id: `stele.testnet`, `demo.stele.testnet`, or a 64-hex implicit account. */
export type AccountId = string;

/** A block to pin a view to: `{ blockId }` or `{ finality }`. Re-exported for callers. */
export type QueryBlock = BlockReference;

/**
 * The read surface every function in this package takes.
 *
 * Structural on purpose: any object with the two provider methods works, so a test can pass a
 * hand-rolled reader, a browser can pass a wallet's provider, and a server can pass `readerFromEnv()`.
 */
export type Reader = Pick<JsonRpcProvider, 'callFunction' | 'viewBlock'>;

/**
 * One state-changing call, ready to sign.
 *
 * `deposit` and `gas` are decimal strings because that is what both `near-api-js` and wallet
 * adapters accept without a float ever touching a money value. `deposit: '1'` is the one-yocto
 * guard on methods that only assert it (`nft_approve`, `list_for_rent`, `set_editor`); `'0'` on
 * non-payable methods (`edit`, `claim_rent`, `end_rental`).
 */
export interface CallRequest {
  contractId: AccountId;
  methodName: string;
  args: Record<string, unknown>;
  deposit: string;
  gas?: string;
}

/** The `@hot-labs/near-connect` call shape, which names the method `method` rather than `methodName`. */
export interface WalletCall {
  contractId: AccountId;
  method: string;
  args: Record<string, unknown>;
  deposit: string;
  gas?: string;
}

/** Thrown when a required environment variable is missing, naming the variable and its purpose. */
export class MissingEnvError extends Error {
  constructor(readonly variable: string, purpose: string) {
    super(`stele/near: ${variable} is not set — ${purpose}`);
    this.name = 'MissingEnvError';
  }
}

/** NEAR account ids are 2..64 characters of lowercase alphanumerics, `-`, `_` and `.` separators. */
const ACCOUNT_ID_PATTERN = /^(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+$/;

/**
 * Rejects an account id before it becomes an RPC error.
 *
 * Worth a local check: a typo in `NEAR_SITE_ACCOUNT` otherwise surfaces as `AccountDoesNotExist`
 * from a node, which reads like a deployment problem rather than a configuration one.
 */
export function assertValidAccountId(accountId: string): void {
  if (accountId.length < 2 || accountId.length > 64 || !ACCOUNT_ID_PATTERN.test(accountId)) {
    throw new RangeError(`stele/near: "${accountId}" is not a NEAR account id`);
  }
}

/** Reads a required environment variable or throws `MissingEnvError`. */
export function requiredEnv(
  env: Record<string, string | undefined>,
  variable: string,
  purpose: string,
): string {
  const value = env[variable];
  if (value === undefined || value.trim() === '') throw new MissingEnvError(variable, purpose);
  return value;
}

/** A provider for one RPC endpoint. `NEAR_RPC_URL` is the only thing that decides the network. */
export function readerFromUrl(rpcUrl: string): Reader {
  if (rpcUrl.trim() === '') throw new Error('stele/near: an RPC URL is required to read the chain');
  return new JsonRpcProvider({ url: rpcUrl });
}

/** A reader from `NEAR_RPC_URL`. */
export function readerFromEnv(env: Record<string, string | undefined> = process.env): Reader {
  return readerFromUrl(requiredEnv(env, 'NEAR_RPC_URL', 'the JSON-RPC endpoint the SDK reads'));
}

/**
 * An account that can sign, for scripts and the sandbox harness.
 *
 * @param privateKey The full key string, e.g. `ed25519:…`. Use a key that is throwaway on any
 *   network that holds value; nothing here prompts, encrypts or stores it.
 */
export function accountFromKey(options: {
  accountId: AccountId;
  privateKey: KeyPairString;
  rpcUrl: string;
}): Account {
  assertValidAccountId(options.accountId);
  return new Account(options.accountId, options.rpcUrl, options.privateKey);
}

/** A signing account from `NEAR_ACCOUNT_ID`, `NEAR_PRIVATE_KEY` and `NEAR_RPC_URL`. */
export function accountFromEnv(env: Record<string, string | undefined> = process.env): Account {
  return accountFromKey({
    accountId: requiredEnv(env, 'NEAR_ACCOUNT_ID', 'the account that signs transactions'),
    privateKey: requiredEnv(env, 'NEAR_PRIVATE_KEY', 'the signing key for NEAR_ACCOUNT_ID') as KeyPairString,
    rpcUrl: requiredEnv(env, 'NEAR_RPC_URL', 'the JSON-RPC endpoint the SDK writes to'),
  });
}

/**
 * Calls one view method and returns its decoded JSON.
 *
 * `undefined` from the provider means the contract returned no bytes at all, which is a wrong
 * method name or a wrong contract far more often than it is an empty result; a method that
 * legitimately returns nothing serializes as `null` and is passed through.
 */
export async function readView<T>(
  reader: Reader,
  contractId: AccountId,
  method: string,
  args: Record<string, unknown> = {},
  blockQuery?: QueryBlock,
): Promise<T> {
  const result: unknown = await reader.callFunction({ contractId, method, args, blockQuery });
  if (result === undefined) {
    throw new Error(
      `stele/near: ${contractId}.${method} returned no value — check the method name and its args`,
    );
  }
  return result as T;
}

/** A block the chain has finalized, as the clock every deadline and expiry is judged against. */
export interface ChainClock {
  height: number;
  /** `block_timestamp` in nanoseconds. Doubles carry about 256 ns of error at current epoch values. */
  timestampNs: bigint;
}

/**
 * The chain's own clock.
 *
 * A deadline built from `Date.now()` is a different number from the one `block_timestamp()`
 * compares against, and a client whose clock is minutes slow gets a rejected transaction with no
 * client-side explanation. Every deadline in this package is built from this read.
 */
export async function readClock(reader: Reader): Promise<ChainClock> {
  const block = await reader.viewBlock({ finality: 'final' });
  return {
    height: block.header.height,
    timestampNs: BigInt(Math.trunc(block.header.timestamp)),
  };
}

/** 30 TGas, the same default wallet adapters use, for calls that do not set their own. */
export const DEFAULT_GAS = 30_000_000_000_000n;

/**
 * Signs and submits one request with a local account, waiting for final execution.
 *
 * For scripts and the sandbox harness. A browser app should hand `toWalletCall(request)` to the
 * wallet instead, so the user sees what they are approving.
 */
export async function submit(account: Account, request: CallRequest): Promise<RpcTransactionResponse> {
  return account.signAndSendTransaction({
    receiverId: request.contractId,
    actions: [
      actions.functionCall(
        request.methodName,
        request.args,
        request.gas === undefined ? DEFAULT_GAS : BigInt(request.gas),
        BigInt(request.deposit),
      ),
    ],
  });
}

/**
 * A `CallRequest` in the shape `near-connect`'s `callFunction` takes.
 *
 * The two differ only in the field name for the method (`methodName` for `near-api-js`,
 * `method` for the wallet), and that mismatch is exactly the kind of thing that is silently
 * ignored by a wallet rather than rejected.
 */
export function toWalletCall(request: CallRequest): WalletCall {
  return {
    contractId: request.contractId,
    method: request.methodName,
    args: request.args,
    deposit: request.deposit,
    ...(request.gas === undefined ? {} : { gas: request.gas }),
  };
}
