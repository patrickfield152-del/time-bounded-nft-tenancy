/**
 * Slot identity: the readable dotted key IS the token id.
 * - token_id = "hero.headline" is stored as-is. The origin hashed the key
 *   with `uint256(keccak256(bytes(key)))`; NEAR stores a `String`, so hashing
 *   bought nothing and hid the key from the indexer. `slotKey` is now a
 *   validator, not a hash.
 * - A key is public. Anyone who reads the chain learns the page's structure. The site's config publishes it anyway.
 *   `registered` keeps unlisted keys unbuyable until the owner registers them.
 * - A `LookupMap` has no range, so the site carries a `registered` vector and
 *   `registered_slots_page` for indexers and marketplaces.
 * - There is no reverse function. A site reads its keys from its own config.
 */

/**
 * Slot keys are namespaced dotted paths: `hero.headline`, `nav.link.1`, `footer.cta`.
 * Lowercase only, and never leading/trailing separators: a key is an on-chain identity that will
 * outlive several front ends, and `Hero.Headline` beside `hero.headline` is two positions selling
 * the same pixels.
 */
const KEY_PATTERN = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/;

/** Long enough for any real path, short enough that a key is never a payload. */
export const MAX_KEY_LENGTH = 128;

export class InvalidSlotKeyError extends Error {
  constructor(
    readonly key: string,
    reason: string,
  ) {
    super(`stele/keys: "${key}" is not a slot key — ${reason}`);
    this.name = 'InvalidSlotKeyError';
  }
}

/**
 * Validates a slot key without rewriting it. Exposed because a config loader wants to report every
 * bad key at once rather than throwing on the first.
 */
export function assertValidSlotKey(key: string): void {
  if (typeof key !== 'string' || key.length === 0) {
    throw new InvalidSlotKeyError(String(key), 'keys are non-empty strings');
  }
  if (key.length > MAX_KEY_LENGTH) {
    throw new InvalidSlotKeyError(key, `keys are at most ${MAX_KEY_LENGTH} characters`);
  }
  if (!KEY_PATTERN.test(key)) {
    throw new InvalidSlotKeyError(
      key,
      'keys are lowercase alphanumeric segments joined by ".", "_" or "-", with no leading or trailing separator',
    );
  }
}

/**
 * The token id for a slot. Validates, then returns the key unchanged.
 * Kept as a function rather than a cast so every call site reads as "this string is an identity",
 * and so a future change to the identity scheme has one place to land.
 */
export function slotKey(key: string): string {
  assertValidSlotKey(key);
  return key;
}

/**
 * Validates many keys and reports ALL failures together.
 * Duplicates are rejected here rather than by the contract: `register_slots` writes the first
 * occurrence and reverts on the second, which leaves the batch half-applied and a `floors` array
 * whose length no longer matches anything.
 */
export function slotKeys(keys: readonly string[]): string[] {
  const failures: string[] = [];
  const seen = new Set<string>();
  const out: string[] = [];

  for (const key of keys) {
    try {
      assertValidSlotKey(key);
    } catch (cause) {
      failures.push(cause instanceof Error ? cause.message : String(cause));
      continue;
    }
    if (seen.has(key)) {
      failures.push(`stele/keys: "${key}" appears more than once in one batch`);
      continue;
    }
    seen.add(key);
    out.push(key);
  }

  if (failures.length > 0) {
    throw new InvalidSlotKeyError(
      keys.join(', '),
      `${failures.length} invalid key(s):\n  ${failures.join('\n  ')}`,
    );
  }
  return out;
}
