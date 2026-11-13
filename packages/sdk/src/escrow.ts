/**
 * The escrow vault — one vault per board, outside the site contract.
 *
 * A vault's income arrives by being the site's treasury: `sweep_treasury()` is permissionless and
 * always pays the pinned treasury, and `withdraw_for(vault)` is permissionless too, so `book()` is
 * two cross-contract calls that move the site's whole cut into the vault. What those calls
 * delivered is measured as a balance delta on the callback — never trusted from the return value.
 *
 * **`claimable` cannot be a local view.** The origin read the site's own
 * `slotOf`/`ownerOfOrZero`/`treasuryBalance`/`pendingWithdrawals(vault)` inside the vault, which a
 * NEAR contract cannot do synchronously. So the vault caches the site legs at each book
 * (`claimable_local`) and `readClaimable` composes that cache with the live site legs in parallel.
 * The composed number is what a claim may drain, capped at the position's last price; whether the
 * claim is ALLOWED (dark, past the delay, not already claimed this episode) is the contract's
 * decision and its own guards.
 */
import { slotKey } from './keys';
import { readView, type AccountId, type CallRequest, type Reader } from './near';
import { fromU128, toU128, type Amount, type U128 } from './settlement';
import type { SlotRecord } from './reads';
import { jsonU64 } from './wire';

/** The one-yocto guard `nft_approve` asserts. It is not a storage charge and is not refunded. */
const ONE_YOCTO = '1';
/** Non-payable methods: `book`, `release`, `claim`, `mark_dark` are all cranks. */
const NO_DEPOSIT = '0';

/**
 * Approves the vault to surrender one position, and tells it the approval id.
 *
 * NEP-178 approves **per token**, not per collection: there is no `setApprovalForAll` on NEAR. The
 * `msg` is what makes the vault record `approval_id`, because `nft_approve` returns it to the
 * CALLER and the vault is not the caller — the site fires `nft_on_approve(token_id, owner_id,
 * approval_id, msg)` at the approved account, fire-and-forget, and the vault stores it.
 *
 * A standing approval is not caller consent: `claim` still asserts the predecessor is the holder.
 */
export function buildApproveForVault(options: {
  site: AccountId;
  tokenId: string;
  vault: AccountId;
}): CallRequest {
  return {
    contractId: options.site,
    methodName: 'nft_approve',
    args: { token_id: slotKey(options.tokenId), account_id: options.vault, msg: '' },
    deposit: ONE_YOCTO,
  };
}

/**
 * Books everything the site currently owes the vault. `book()` on the vault is the crank; the
 * booking happens on the callback from its two cross-contract calls, so this request's own outcome
 * says nothing about how much was booked.
 */
export function buildBook(options: { vault: AccountId }): CallRequest {
  return {
    contractId: options.vault,
    methodName: 'book',
    args: {},
    deposit: NO_DEPOSIT,
  };
}

/**
 * Releases one mature deposit to its beneficiary. Maturity is evaluated as of `dark_since`, so a
 * deposit that had matured before the lights went out is the publisher's whatever a slow keeper did.
 */
export function buildRelease(options: { vault: AccountId; depositId: bigint | number }): CallRequest {
  const id = typeof options.depositId === 'number' ? BigInt(Math.trunc(options.depositId)) : options.depositId;
  if (id < 0n) throw new RangeError('stele/escrow: a deposit id cannot be negative');
  return {
    contractId: options.vault,
    methodName: 'release',
    args: { deposit_id: jsonU64(id, 'deposit_id') },
    deposit: NO_DEPOSIT,
  };
}

/**
 * Claims a position whose publisher has gone dark: three phases on the contract side — the same
 * two-call book, the guards, then a surrender of the token to the vault itself. Crediting happens
 * in a private callback, so a failed transfer credits nothing.
 *
 * `minPaid` is the claimer's floor: the transaction reverts rather than surrendering a position for
 * less.
 */
export function buildClaim(options: { vault: AccountId; tokenId: string; minPaid: Amount }): CallRequest {
  return {
    contractId: options.vault,
    methodName: 'claim',
    args: { token_id: slotKey(options.tokenId), min_paid: toU128(options.minPaid) },
    deposit: NO_DEPOSIT,
  };
}

/**
 * Starts the dark clock. Factory-gated: the factory is the attestor's proxy, so this is called by
 * the factory after the attestor names the vault, never by the vault's own operator.
 */
export function buildMarkDark(options: { vault: AccountId }): CallRequest {
  return {
    contractId: options.vault,
    methodName: 'mark_dark',
    args: {},
    deposit: NO_DEPOSIT,
  };
}

/** One vault's public state, from the four views that describe it. */
export interface VaultState {
  /** While dark, maturity is evaluated as of the moment the lights went out. */
  isDark: boolean;
  /** Inbound the vault holds that is neither booked, credited nor reserve. */
  unbooked: bigint;
  /** The sum of deposits the publisher has NOT yet earned — what a claim may drain. */
  immaturePool: bigint;
  depositCount: number;
}

export async function readVault(reader: Reader, vault: AccountId): Promise<VaultState> {
  const [isDark, unbooked, immaturePool, depositCount] = await Promise.all([
    readView<boolean>(reader, vault, 'is_dark'),
    readView<U128>(reader, vault, 'unbooked'),
    readView<U128>(reader, vault, 'immature_pool'),
    readView<number>(reader, vault, 'deposit_count'),
  ]);
  return {
    isDark,
    unbooked: fromU128(unbooked),
    immaturePool: fromU128(immaturePool),
    depositCount,
  };
}

/** A deposit, with the vault's own maturity verdict beside it. */
export interface VaultDeposit {
  id: number;
  /** Remaining, undrained. Zero once released or fully claimed. */
  amount: bigint;
  bookedAtNs: bigint;
  darkAtBookingNs: bigint;
  /** Who owned the board when this was booked, and who `release` pays. */
  beneficiary: AccountId;
  /** The vault's verdict — releasable and never claimable. */
  mature: boolean;
}

interface DepositRecord {
  amount: U128;
  booked_at: number;
  dark_at_booking: number;
  beneficiary: AccountId;
}

/**
 * Every deposit on a vault, with the vault's own maturity verdict beside each.
 *
 * Read from the contract rather than recomputed here: the predicate decides both sides of every
 * dispute, and a TypeScript twin of it would be a second implementation with no shared vector.
 */
export async function readDeposits(reader: Reader, vault: AccountId): Promise<VaultDeposit[]> {
  const count = await readView<number>(reader, vault, 'deposit_count');
  const ids = Array.from({ length: count }, (_, index) => index);
  return Promise.all(
    ids.map(async (id) => {
      const [record, mature] = await Promise.all([
        readView<DepositRecord | null>(reader, vault, 'deposit_at', { id }),
        readView<boolean>(reader, vault, 'mature', { id }),
      ]);
      if (record === null) {
        throw new Error(`stele/escrow: ${vault}.deposit_at(${id}) is empty inside deposit_count ${count}`);
      }
      return {
        id,
        amount: fromU128(record.amount),
        bookedAtNs: BigInt(Math.trunc(record.booked_at)),
        darkAtBookingNs: BigInt(Math.trunc(record.dark_at_booking)),
        beneficiary: record.beneficiary,
        mature,
      };
    }),
  );
}

/**
 * What `claim` could drain from a vault for one position, and the four legs it is made of.
 *
 * `cached` is the vault's own `claimable_local` — the site legs as they stood at the last book.
 * The three live legs are what a claim's own book will turn into a fresh, immature deposit before
 * the drain runs, which is why they count here even though the vault does not hold them yet.
 *
 * `claimable` is `min(available, lastPrice)` and nothing more. It is deliberately not a prediction
 * that a claim will succeed: the dark clock, the claim delay, the once-per-episode rule and the
 * holder assertion are the contract's, and a client that reimplemented them would be a second
 * authority on when a board may be taken.
 */
export interface Claimable {
  cached: bigint;
  siteTreasury: bigint;
  sitePending: bigint;
  unbooked: bigint;
  /** The four legs summed, before the cap. */
  available: bigint;
  /** The position's last price: the most a claim can ever credit. */
  lastPrice: bigint;
  /** `min(available, lastPrice)`. */
  claimable: bigint;
}

export async function readClaimable(
  reader: Reader,
  refs: { vault: AccountId; site: AccountId },
  tokenId: string,
): Promise<Claimable> {
  const token = slotKey(tokenId);
  const [cached, siteTreasury, sitePending, unbooked, slot] = await Promise.all([
    readView<U128>(reader, refs.vault, 'claimable_local', { token_id: token }),
    readView<U128>(reader, refs.site, 'treasury_balance'),
    readView<U128>(reader, refs.site, 'pending_withdrawals', { account: refs.vault }),
    readView<U128>(reader, refs.vault, 'unbooked'),
    readView<SlotRecord | null>(reader, refs.site, 'slot_of', { token_id: token }),
  ]);

  const legs = {
    cached: fromU128(cached),
    siteTreasury: fromU128(siteTreasury),
    sitePending: fromU128(sitePending),
    unbooked: fromU128(unbooked),
  };
  const available = legs.cached + legs.siteTreasury + legs.sitePending + legs.unbooked;
  const lastPrice = slot === null ? 0n : fromU128(slot.last_price);

  return {
    ...legs,
    available,
    lastPrice,
    claimable: available > lastPrice ? lastPrice : available,
  };
}
