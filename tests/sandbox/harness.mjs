//! Local NEAR sandbox harness: one ephemeral nearcore process, a `test.near`
//! root account, and the handful of RPC primitives the stele flows need.
//!
//! Zero real funds. `near-sandbox` downloads its pinned nearcore binary once
//! and spawns it with a throwaway home directory; every account and every
//! token used by the tests is created inside that process. The provider URL is
//! always `http://127.0.0.1:<random port>`; nothing here can reach mainnet.
//!
//! The sandbox-only RPC methods (`sandbox_fast_forward`, `sandbox_patch_state`)
//! are not in near-api-js' generated `Methods` interface. They are valid at
//! runtime, so the wrapper casts the method name to `never`, the same
//! `sendJsonRpc("sandbox_fast_forward" as never, ...)` escape a TypeScript
//! caller needs.

import { existsSync } from "node:fs";
import { access, readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import {
  Account,
  JsonRpcProvider,
  KeyPair,
  getTransactionLastResult,
  nearToYocto,
} from "near-api-js";
import { DEFAULT_ACCOUNT_ID, DEFAULT_PRIVATE_KEY, Sandbox } from "near-sandbox";

/**
 * The pre-funded root account every sandbox starts with. near-sandbox's
 * genesis also carries a `test.near` account, but its key is generated per
 * home and never written to disk; `sandbox` is the one whose key the package
 * publishes, so it is the only account a harness can sign for from scratch.
 */
export const ROOT_ACCOUNT = DEFAULT_ACCOUNT_ID;
/** near-sandbox's default nearcore build (package 0.3.5). */
export const SANDBOX_VERSION = process.env.NEAR_SANDBOX_VERSION ?? "2.14.0";
/** 1 NEAR in yoctoNEAR. */
export const ONE_NEAR = 10n ** 24n;
/** 1 TGas in gas units; near-api-js takes gas units, not TGas. */
export const ONE_TGAS = 10n ** 12n;
/** Default call gas: enough for the factory's deploy/init batches. */
export const DEFAULT_GAS = 300n * ONE_TGAS;

const require = createRequire(import.meta.url);

/** Resolve the near-sandbox package root without relying on package exports. */
function nearSandboxRoot() {
  let dir = dirname(require.resolve("near-sandbox"));
  for (;;) {
    if (existsSync(join(dir, "package.json"))) return dir;
    const parent = dirname(dir);
    if (parent === dir) {
      throw new Error("could not locate the near-sandbox package root");
    }
    dir = parent;
  }
}

/**
 * The binary `near-sandbox` downloads on first use. `Sandbox.start` creates
 * the path; `NEAR_SANDBOX_BIN_PATH` / `DIR_TO_DOWNLOAD_BINARY` keep the same
 * overrides the package itself honours.
 */
export function sandboxBinaryPath() {
  const fromEnv = process.env.NEAR_SANDBOX_BIN_PATH;
  if (fromEnv) return fromEnv;
  const base =
    process.env.DIR_TO_DOWNLOAD_BINARY ?? join(nearSandboxRoot(), "bin");
  return join(base, `near-sandbox-${SANDBOX_VERSION}`, "near-sandbox");
}

/** `"neard"` for 1, `"1.5"` for 1.5: accept decimal NEAR or yocto bigint. */
function toYocto(near) {
  return typeof near === "bigint" ? near : nearToYocto(String(near));
}

/** Decode one `SuccessValue` base64 string; near-api-js only decodes the top one. */
function decodeSuccessValue(value) {
  if (typeof value !== "string") return null;
  const bytes = Uint8Array.from(Buffer.from(value, "base64"));
  const text = new TextDecoder().decode(bytes);
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

/** Receipt status in one word: `Success`, `SuccessReceiptId`, or the failure. */
export function receiptStatus(receipt) {
  const status = receipt?.outcome?.status;
  if (status === "Unknown") return "Unknown";
  if (typeof status === "string") return status;
  if (status && "SuccessValue" in status) return "SuccessValue";
  if (status && "SuccessReceiptId" in status) return "SuccessReceiptId";
  if (status && "Failure" in status) {
    return `Failure: ${JSON.stringify(status.Failure)}`;
  }
  return `Unrecognised: ${JSON.stringify(status)}`;
}

/** Human-readable status for every receipt produced by one call. */
export function receiptSummary(tx) {
  if (!tx || !Array.isArray(tx.receipts) || tx.receipts.length === 0) {
    return "no receipts";
  }
  return tx.receipts
    .map((receipt, index) => `#${index} ${receiptStatus(receipt)}`)
    .join(", ");
}

export class SandboxHarness {
  #sandbox;
  #provider;
  #accounts;
  #root;
  #stopped;

  constructor(sandbox, provider, root) {
    this.#sandbox = sandbox;
    this.#provider = provider;
    this.#root = root;
    this.#accounts = new Map([[ROOT_ACCOUNT, root]]);
    this.#stopped = false;
  }

  /**
   * Start nearcore, create `test.near` from near-sandbox's built-in genesis
   * key, and return a ready harness.
   */
  static async start({ version = SANDBOX_VERSION } = {}) {
    if (process.platform === "win32") {
      throw new Error(
        "near-sandbox ships Linux/macOS binaries only; run the dry-run under WSL2",
      );
    }
    // `test.near` is already in near-sandbox 0.3.5's default genesis (with the
    // default key below). Adding it again fails genesis validation with
    // "Duplicate account id test.near", so only the epoch length is tweaked.
    const sandbox = await Sandbox.start({
      version,
      config: {
        // Fast-forward crosses epoch boundaries in normal block-production
        // chunks; a long epoch keeps the 1-hour rental expiry cheap to reach.
        additionalGenesis: { epoch_length: 1_000 },
      },
    });
    try {
      const provider = new JsonRpcProvider({ url: sandbox.rpcUrl });
      const root = new Account(ROOT_ACCOUNT, provider, DEFAULT_PRIVATE_KEY);
      return new SandboxHarness(sandbox, provider, root);
    } catch (error) {
      await sandbox.tearDown().catch(() => {});
      throw error;
    }
  }

  get provider() {
    return this.#provider;
  }

  get root() {
    return this.#root;
  }

  /** RPC endpoint, for logs only. */
  get rpcUrl() {
    return this.#sandbox.rpcUrl;
  }

  /** Temporary nearcore home directory (removed by `stop`). */
  get homeDir() {
    return this.#sandbox.homeDir;
  }

  /**
   * Raw JSON-RPC. The sandbox methods are outside near-api-js' `Methods`
   * interface; the `never` cast is the TypeScript escape hatch.
   */
  async rpc(method, params = {}) {
    const send = this.#provider.sendJsonRpc.bind(this.#provider);
    return send(/** @type {never} */ (method), params);
  }

  /** Current block timestamp in nanoseconds, the unit every site deadline uses. */
  async nowNs() {
    const block = await this.rpc("block", { finality: "final" });
    return BigInt(block.header.timestamp);
  }

  /** Create `id` (or `<id>.test.near`) as a root sub-account and return its signer. */
  async createAccount(id, near = "100") {
    const suffix = `.${ROOT_ACCOUNT}`;
    const qualified = id.endsWith(suffix) ? id : `${id}${suffix}`;
    if (this.#accounts.has(qualified)) return this.#accounts.get(qualified);
    const prefix = qualified.slice(0, -suffix.length);
    if (prefix.length === 0) {
      throw new Error(`cannot create the root account: ${id}`);
    }
    const keyPair = KeyPair.fromRandom("ed25519");
    await this.#root.createSubAccount({
      accountOrPrefix: prefix,
      publicKey: keyPair.getPublicKey(),
      nearToTransfer: toYocto(near),
    });
    const account = new Account(qualified, this.#provider, keyPair.toString());
    account.keyPair = keyPair;
    this.#accounts.set(qualified, account);
    return account;
  }

  /** Existing signer for `id`; throws rather than inventing one. */
  account(id) {
    const account = this.#accounts.get(id);
    if (!account) {
      throw new Error(`unknown sandbox account: ${id} (create it first)`);
    }
    return account;
  }

  /**
   * Top up an existing account with a single `sandbox_patch_state` Account
   * record. The record preserves code hash and storage usage; the root account
   * is deliberately off limits so a patch can never rewrite genesis funds.
   */
  async patchAccount(id, near) {
    if (id === ROOT_ACCOUNT) {
      throw new Error("patchAccount refuses to overwrite the root account");
    }
    const current = await this.#provider.viewAccount({ accountId: id });
    const amount = (current.amount + toYocto(near)).toString();
    await this.rpc("sandbox_patch_state", {
      records: [
        {
          Account: {
            account_id: id,
            account: {
              amount,
              locked: current.locked.toString(),
              code_hash: current.code_hash,
              storage_usage: current.storage_usage,
            },
          },
        },
      ],
    });
  }

  /** Advance the chain by `deltaHeight` blocks (and their block time). */
  async fastForward(deltaHeight) {
    const height = Number(deltaHeight);
    if (!Number.isSafeInteger(height) || height <= 0) {
      throw new Error(`fastForward needs a positive integer, got ${deltaHeight}`);
    }
    await this.rpc("sandbox_fast_forward", { delta_height: height });
  }

  /**
   * Call a contract method and return the decoded value plus every receipt.
   * `gas` is gas units (not TGas); `deposit` is yoctoNEAR.
   */
  async call(
    signer,
    contractId,
    method,
    args = {},
    { gas = DEFAULT_GAS, deposit = 0n, waitUntil = "FINAL" } = {},
  ) {
    let response;
    try {
      response = await signer.callFunctionRaw({
        contractId,
        methodName: method,
        args,
        gas: typeof gas === "bigint" ? gas : BigInt(gas),
        deposit: typeof deposit === "bigint" ? deposit : BigInt(deposit),
        waitUntil,
      });
    } catch (error) {
      const receipts = error?.transactionResult?.receipts_outcome ?? [];
      console.error(
        "[sandbox] failed receipts",
        JSON.stringify(
          receipts.map((r, i) => ({
            i,
            gas: r.outcome.gas_burnt,
            status: r.outcome.status,
            logs: (r.outcome.logs ?? []).slice(-2),
          })),
          null,
          2,
        ),
      );
      throw error;
    }
    const receipts = Array.isArray(response.receipts_outcome)
      ? response.receipts_outcome
      : [];
    let result = getTransactionLastResult(response);
    if (result === null) {
      for (let index = receipts.length - 1; index >= 0; index -= 1) {
        const status = receipts[index].outcome.status;
        if (status && typeof status === "object" && "SuccessValue" in status) {
          result = decodeSuccessValue(status.SuccessValue);
          break;
        }
      }
    }
    return {
      result,
      receipts,
      response,
      status: response.status,
      logs: receipts.flatMap((receipt) => receipt.outcome.logs ?? []),
    };
  }

  /** Read-only call through `JsonRpcProvider.callFunction`. */
  async view(contractId, method, args = {}) {
    const result = await this.#provider.callFunction({
      contractId,
      method,
      args,
    });
    if (result === undefined) {
      throw new Error(`view ${contractId}.${method} returned no value`);
    }
    return result;
  }

  /**
   * Deploy a wasm file (optionally initialising it). A missing account is
   * created from the root first, so contract accounts are never pre-existing.
   */
  async deploy(wasmPath, accountId, initMethod, initArgs = {}) {
    const wasm = await readFile(wasmPath);
    let account = this.#accounts.get(accountId);
    if (!account) account = await this.createAccount(accountId, "100");
    const deployed = await account.deployContract(new Uint8Array(wasm));
    let init = null;
    if (initMethod) {
      init = await this.call(account, accountId, initMethod, initArgs);
    }
    return { account, deployed, init, wasmBytes: wasm.length };
  }

  /** Kill the node and remove its temporary home directory. Idempotent. */
  async stop() {
    if (this.#stopped) return;
    this.#stopped = true;
    await this.#sandbox.tearDown();
  }
}

/** Verify the package-managed binary exists after a start; used by the runner. */
export async function assertSandboxBinary() {
  const path = sandboxBinaryPath();
  await access(path);
  return path;
}
