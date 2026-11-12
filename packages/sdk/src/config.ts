/**
 * A site's configuration, and where its addresses come from.
 *
 * **Accounts come from the environment, never from this file.** There is no chain list and no
 * deployed-address table: `readNetwork()` reads `NEAR_RPC_URL` and the four accounts of the
 * deployment plan, so the same build points at testnet, mainnet or a local sandbox without a
 * version bump. A missing variable throws `MissingEnvError` naming itself — a typo in
 * `NEAR_SITE_ACCOUNT` otherwise arrives as `AccountDoesNotExist` from a node, which reads like a
 * deployment failure rather than a configuration one.
 *
 * The `SiteConfig` half is client-side only: names, kinds and floors that a renderer and a deploy
 * call need. **It is deliberately data, not functions** — it crosses the server/client boundary in
 * a Next app, and a function there is an error rather than a prop. The content gateway is a
 * template string for the same reason.
 */
import { ContentKind } from './content';
import { slotKeys } from './keys';
import {
  assertValidAccountId,
  readerFromUrl,
  requiredEnv,
  runtimeEnv,
  type AccountId,
  type Reader,
} from './near';
import { parseUnits, toU128, type U128 } from './settlement';

/**
 * Decimals a floor string is parsed against when the config does not say otherwise.
 *
 * 24 is native NEAR's yocto count. A 6-decimal settlement token is the common demo case, and
 * parsing a floor for it against 24 would be off by 18 orders of magnitude — which is why
 * `parseFloor` takes the decimals explicitly and `defineSite` carries them through.
 */
export const DEFAULT_SETTLEMENT_DECIMALS = 24;

/** Parses a human decimal amount against a settlement asset's decimals. */
export function parseFloor(amount: string, decimals: number = DEFAULT_SETTLEMENT_DECIMALS): bigint {
  return parseUnits(amount, decimals);
}

/**
 * How `@stele/react` should render a slot. Purely a client-side convention — the chain stores an
 * opaque hash and knows nothing about kinds, and a site rendering its own payload structure through
 * `useSlot()` can ignore this entirely.
 */
export type SlotKindName = 'text' | 'link' | 'image' | 'video';

export const CONTENT_KIND_BY_NAME: Record<SlotKindName, number> = {
  text: ContentKind.Text,
  link: ContentKind.Link,
  image: ContentKind.Image,
  video: ContentKind.Video,
};

/**
 * Default gateway: a per-CID subdomain, so the CID is the whole address.
 *
 * A template rather than a function because a site config crosses the server/client boundary in a
 * Next app, and functions do not. `{cid}` is replaced; a template without it is treated as a base
 * URL and joined.
 */
export const DEFAULT_CONTENT_GATEWAY = 'https://{cid}.ipfs.w3s.link/';

/** Resolves a CID through a gateway template. */
export function contentUrl(gateway: string, cid: string): string {
  if (gateway.includes('{cid}')) return gateway.replaceAll('{cid}', cid);
  return `${gateway.replace(/\/+$/, '')}/${cid}`;
}

export class InvalidSiteConfigError extends Error {
  constructor(message: string) {
    super(`stele/config: ${message}`);
    this.name = 'InvalidSiteConfigError';
  }
}

export interface SlotDefinitionInput {
  /** The dotted key, e.g. `hero.headline`. It is also the on-chain token id. */
  key: string;
  /** Render hint. Defaults to `text`. */
  kind?: SlotKindName;
  /** Initial floor, as a decimal string in the settlement asset's units, or base units. */
  floor: string | bigint;
}

export interface SlotDefinition {
  key: string;
  kind: SlotKindName;
  /** The payload kind a renderer should expect until content says otherwise. */
  contentKind: number;
  /** Base units of the settlement asset. */
  floor: bigint;
}

export interface DefineSiteInput {
  name: string;
  symbol: string;
  /** The site's contract account, e.g. `NEAR_SITE_ACCOUNT`. */
  site: AccountId;
  /** The NEP-141 the site settles in, or `null` for native NEAR. */
  settlementFt?: AccountId | null;
  /** Content base URI for token metadata; stored in `ContractMetadata`. */
  baseUri?: string;
  /** Decimal count floors are parsed against. Defaults to `DEFAULT_SETTLEMENT_DECIMALS`. */
  decimals?: number;
  /** Where a reader resolves a CID: a template with `{cid}`, or a base URL to join. */
  gateway?: string;
  slots: readonly SlotDefinitionInput[];
}

/** A site's client-side configuration, normalized. */
export interface SiteConfig {
  name: string;
  symbol: string;
  site: AccountId;
  settlementFt: AccountId | null;
  baseUri: string;
  /** The gateway template this site reads content through. */
  gateway: string;
  slots: readonly SlotDefinition[];
  slotsByKey: Readonly<Record<string, SlotDefinition>>;
  keys: readonly string[];
}

/**
 * Validates a site config and normalizes it into the shape every consumer wants.
 *
 * Every bad key is reported at once rather than on the first throw: a config with three typos
 * should cost one edit, not three build cycles.
 */
export function defineSite(input: DefineSiteInput): SiteConfig {
  const name = input.name.trim();
  const symbol = input.symbol.trim();
  if (name === '') throw new InvalidSiteConfigError('a site needs a name');
  if (symbol === '') throw new InvalidSiteConfigError('a site needs a symbol');

  assertValidAccountId(input.site);
  const settlementFt = input.settlementFt ?? null;
  if (settlementFt !== null) assertValidAccountId(settlementFt);

  if (input.slots.length === 0) {
    throw new InvalidSiteConfigError('a site needs at least one slot');
  }

  const keys = slotKeys(input.slots.map((slot) => slot.key));
  const decimals = input.decimals ?? DEFAULT_SETTLEMENT_DECIMALS;
  const gateway = input.gateway ?? DEFAULT_CONTENT_GATEWAY;

  const slots: SlotDefinition[] = input.slots.map((slot, index) => {
    const kind = slot.kind ?? 'text';
    const contentKind = CONTENT_KIND_BY_NAME[kind];
    if (contentKind === undefined) {
      throw new InvalidSiteConfigError(`"${slot.key}" has unknown kind "${String(kind)}"`);
    }
    const floor = typeof slot.floor === 'bigint' ? slot.floor : parseUnits(slot.floor, decimals);
    if (floor < 0n) throw new InvalidSiteConfigError(`"${slot.key}" has a negative floor`);
    // `keys` was validated as a whole batch; `slotKeys` preserves input order, so this
    // re-association is what keeps floors and keys aligned when the batch check reports errors.
    return { key: keys[index]!, kind, contentKind, floor };
  });

  return {
    name,
    symbol,
    site: input.site,
    settlementFt,
    baseUri: input.baseUri ?? '',
    gateway,
    slots,
    slotsByKey: Object.fromEntries(slots.map((slot) => [slot.key, slot])),
    keys,
  };
}

/** The `{ key: floor }` map `buildCreateSite` takes, floors already in wire form. */
export function slotFloors(config: SiteConfig): Record<string, U128> {
  return Object.fromEntries(config.slots.map((slot) => [slot.key, toU128(slot.floor)]));
}

/** The deployment plan's accounts, as the environment names them. */
export interface SteleNetwork {
  rpcUrl: string;
  factory: AccountId;
  site: AccountId;
  vault: AccountId;
  /** `null` when the deployment settles in native NEAR. */
  settlementFt: AccountId | null;
}

/**
 * The deployment, from the environment.
 *
 * `NEAR_FT_ACCOUNT` is optional and its absence means native settlement: a site with a token is a
 * different deployment plan, not a different build.
 */
export function readNetwork(env: Record<string, string | undefined> = runtimeEnv()): SteleNetwork {
  const factory = requiredEnv(env, 'NEAR_FACTORY_ACCOUNT', 'the factory that deploys sites');
  const site = requiredEnv(env, 'NEAR_SITE_ACCOUNT', 'the site this app renders');
  const vault = requiredEnv(env, 'NEAR_VAULT_ACCOUNT', 'the escrow vault bound to that site');
  assertValidAccountId(factory);
  assertValidAccountId(site);
  assertValidAccountId(vault);

  const ft = env.NEAR_FT_ACCOUNT?.trim() ?? '';
  if (ft !== '') assertValidAccountId(ft);

  return {
    rpcUrl: requiredEnv(env, 'NEAR_RPC_URL', 'the JSON-RPC endpoint the SDK reads'),
    factory,
    site,
    vault,
    settlementFt: ft === '' ? null : ft,
  };
}

/** A reader for a network description. Convenience for apps that already called `readNetwork`. */
export function readerFor(network: Pick<SteleNetwork, 'rpcUrl'>): Reader {
  return readerFromUrl(network.rpcUrl);
}
