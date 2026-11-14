/**
 * Escrow vault builders and views: one `vault` per board receives `sweep_treasury()` / `withdraw_for(vault)` via `book()`, and `readClaimable` joins `claimable_local` (`cached`, `claimable`, `slotOf`, `ownerOfOrZero`, `treasuryBalance`, `pendingWithdrawals(vault)`).
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

/** Approves vault to surrender a slot via NEP-178 `nft_approve` (no `setApprovalForAll`; `msg` triggers `nft_on_approve(token_id, owner_id, approval_id, msg)`). */
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

/** Cranks `book()` on the vault to sweep site balances and record a new deposit. */
export function buildBook(options: { vault: AccountId }): CallRequest {
  return {
    contractId: options.vault,
    methodName: 'book',
    args: {},
    deposit: NO_DEPOSIT,
  };
}

/** Releases one mature deposit to its beneficiary (evaluated as of `dark_since`). */
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

/** Surrenders a position during a dark episode (3-phase async claim on-chain; `minPaid` floor). */
export function buildClaim(options: { vault: AccountId; tokenId: string; minPaid: Amount }): CallRequest {
  return {
    contractId: options.vault,
    methodName: 'claim',
    args: { token_id: slotKey(options.tokenId), min_paid: toU128(options.minPaid) },
    deposit: NO_DEPOSIT,
  };
}

/** Starts the dark clock (called via factory/attestor). */
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

/** Reads every vault deposit with the contract's maturity verdict. */
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

/** Claimable amount (`min(available, lastPrice)`) joined from vault cache and live site legs. */
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
