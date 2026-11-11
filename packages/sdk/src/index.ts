// The pricing twin — byte-identical to the contract's `pricing.rs`, and to the origin's Solidity.
export {
  BPS_DENOMINATOR,
  SECONDS_PER_WEEK,
  PricingOverflowError,
  computeTakePrice,
  computeSplit,
  computeElapsedWeeks,
  computeBuyBreakdown,
  resolveReversionBase,
  askCeiling,
} from './pricing';
export type { TakeQuote, Split, SiteEconomics, BuyBreakdown } from './pricing';

// Rent — accrual, the unaccrued remainder a buyer inherits, and the fee split.
export {
  SECONDS_PER_DAY,
  MIN_RENTAL_DURATION,
  accruedOf,
  accrual,
  rentCost,
  rentSplit,
  quoteRent,
} from './rentals';
export type { Accrual, RentSplit, RentQuote } from './rentals';

// Chain plumbing — one reader, and an account only where something must sign.
export {
  MissingEnvError,
  DEFAULT_GAS,
  assertValidAccountId,
  requiredEnv,
  readerFromUrl,
  readerFromEnv,
  accountFromKey,
  accountFromEnv,
  readView,
  readClock,
  submit,
  toWalletCall,
} from './near';
export type { AccountId, QueryBlock, Reader, CallRequest, WalletCall, ChainClock } from './near';

// Money — every wire amount is a decimal string; every conversion is exact.
export {
  toU128,
  fromU128,
  parseUnits,
  formatUnits,
  nearToYocto,
  MIN_STORAGE_TOP_UP,
  NATIVE_NEAR,
  isNativeSettlement,
  readSettlementAsset,
} from './settlement';
export type { U128, Amount, SettlementAsset } from './settlement';

// Content addressing — the hash is the address, and `media_hash` is a different encoding.
export {
  SCHEME_VERSION,
  ContentKind,
  SITE_DEFINED_KIND_MIN,
  HEADER_BYTES,
  MAX_OBJECT_BYTES,
  ContentTooLargeError,
  MalformedContentError,
  encodeContent,
  encodeText,
  encodeLink,
  encodeImage,
  encodeLinked,
  decodeContent,
  decodeLinked,
  readContent,
  sha256Hex,
  mediaHash,
  assertContentHash,
  contentHashToCid,
  cidToContentHash,
  ipfsUri,
  cidFromIpfsUri,
  ipfsGatewayUrl,
} from './content';
export type {
  ContentHash,
  EncodedContent,
  DecodedContent,
  DecodedLinked,
  ContentResult,
  ContentFailure,
} from './content';

// Slot identity — the readable dotted key IS the token id.
export { slotKey, slotKeys, assertValidSlotKey, MAX_KEY_LENGTH, InvalidSlotKeyError } from './keys';

// Reads — one wave of parallel views per page.
export {
  readSlots,
  readSlot,
  readSlotsMulti,
  readSiteTerms,
  economicsFromTerms,
  isTokenSettled,
  readEncumbrance,
  readBuyContext,
  readRental,
  readListing,
  readCanEdit,
  readPendingWithdrawal,
  readTreasuryBalance,
  readRegisteredSlotCount,
  readRegisteredSlotsPage,
  readRoyalty,
} from './reads';
export type {
  SlotState,
  SlotRecord,
  QuoteView,
  ListingRecord,
  RentalRecord,
  TermsView,
  SiteTerms,
  BuyContext,
  Rental,
  Listing,
} from './reads';

// Writes — request objects, never a wallet of our own.
export {
  DEFAULT_DEADLINE_SECS,
  DEFAULT_SLIPPAGE_BPS,
  UnsupportedSettlementError,
  InvalidEconomicsError,
  buildBuy,
  buildBuyFor,
  buildBuyFrom,
  buildEdit,
  buildSetEditor,
  buildListForRent,
  buildRent,
  buildExtendRental,
  buildClaimRent,
  buildEndRental,
  buildWithdrawFor,
  buildSweepTreasury,
  siteConfigOf,
  buildCreateSite,
} from './writes';
export type {
  BuildBuyOptions,
  BuildRentOptions,
  BuildCreateSiteOptions,
  SiteConfigOptions,
  SiteConfigArgs,
  SiteEconomicsConfig,
  SiteRentalConfig,
  SiteFloorPolicyConfig,
} from './writes';

// Escrow — one vault per board; `claimable` is composed from the vault cache plus live site legs.
export {
  buildApproveForVault,
  buildBook,
  buildRelease,
  buildClaim,
  buildMarkDark,
  readVault,
  readDeposits,
  readClaimable,
} from './escrow';
export type { VaultState, VaultDeposit, Claimable } from './escrow';

// Tenancy — a gas-only function-call key, bounded by the contract's clock rather than by the key.
export {
  SESSION_METHODS,
  SESSION_ALLOWANCE_NEAR,
  SESSION_ALLOWANCE_YOCTO,
  newSessionKey,
  issueSessionKey,
  revokeSessionKey,
  sessionAccount,
} from './tenancy';
export type { SessionKey, IssueSessionKeyOptions } from './tenancy';

// Site config — a file a builder edits, and the deployment's accounts from env.
export {
  defineSite,
  contentUrl,
  slotFloors,
  parseFloor,
  readNetwork,
  readerFor,
  DEFAULT_SETTLEMENT_DECIMALS,
  CONTENT_KIND_BY_NAME,
  DEFAULT_CONTENT_GATEWAY,
  InvalidSiteConfigError,
} from './config';
export type {
  SiteConfig,
  SlotDefinition,
  SlotDefinitionInput,
  SlotKindName,
  DefineSiteInput,
  SteleNetwork,
} from './config';
