/**
 * Content addressing — the bytes a slot points at, and the hash the chain holds.
 *
 *     object      = [schemeVersion:u8][kind:u8][payload…]
 *     contentHash = sha256(object)        // 64 lowercase hex characters, stored as a String
 *
 * `payload` is opaque bytes. Not a canonical string with named segments: the framework hashes
 * BYTES, so any structure — JSON for a styled text block, raw bytes for an image — is the caller's
 * concern inside `payload`, and extending that vocabulary never invalidates a hash already on
 * chain.
 *
 * Three decisions carried over from the origin and still load-bearing:
 *
 *   - **sha256, not keccak.** The contract never hashes a page body — it stores the string the
 *     writer submitted and hashes its own tuple for `encumbrance_hash` — so a hash that is also a
 *     CID's multihash buys a backend-free read path. `contentHashToCid` is that payoff.
 *   - **Version and kind live inside the object.** A renderer fetches, verifies, then reads two
 *     bytes and knows what it holds. An unknown version degrades to `fallback` with a warning
 *     instead of the silent blank a bare hash mismatch produces, and `kind` domain-separates text
 *     from an `ipfs://` video reference.
 *   - **Content-only preimage.** No site, no slot key. The chain already binds hash to slot, so
 *     adding them gains nothing and costs deduplication across slots.
 *
 * **Two different base encodings, for two different fields, because NEP-177 says so.** A slot's
 * `page_hash` is a `String` and this package writes lowercase hex. A token's `media` is
 * `ipfs://<CID>` (a multihash, not a bare digest) and its `media_hash` must be the SHA-256 digest
 * of the referenced bytes in **base64** — `mediaHash` is that encoding, and it is not the same
 * function as `sha256Hex`.
 *
 * Sanitization is a RENDER-time concern, not a hash-time one: the renderer treats all content as
 * untrusted and escapes at render.
 */
import { sha256 } from '@noble/hashes/sha2.js';

/** A SHA-256 digest as 64 lowercase hex characters. The contract stores this form directly. */
export type ContentHash = string;

/**
 * Bumping this is how a future object layout ships. It is in the preimage, so a v2 scheme is a
 * version bump rather than an argument.
 */
export const SCHEME_VERSION = 1;

/**
 * Well-known payload kinds. `kind` is a `u8`; ids from `SITE_DEFINED_KIND_MIN` up are the site's
 * and will never be assigned a meaning here.
 *
 * `0` is deliberately not a kind: a zero byte is what an uninitialized buffer and a truncated
 * fetch both look like, and neither should decode as valid content.
 */
export const ContentKind = {
  Text: 1,
  Link: 2,
  Image: 3,
  Video: 4,
  /**
   * A body of some other kind with a destination attached — see `encodeLinked`.
   *
   * A wrapper rather than a product type: a link is optional and can be added to any slot, and a
   * thing that can be added to anything is a decoration on content rather than a kind of it.
   */
  Linked: 5,
} as const;

export type ContentKind = (typeof ContentKind)[keyof typeof ContentKind];

/** Kind ids at or above this are reserved for the site and never interpreted by this package. */
export const SITE_DEFINED_KIND_MIN = 128;

/** The header is two bytes: `[schemeVersion][kind]`. */
export const HEADER_BYTES = 2;

/**
 * **Enforced at encode time, so it bites before anyone signs.**
 *
 * IPFS raw blocks top out around here; past it you need chunked DAG-PB and `sha256(bytes)` stops
 * being the CID — silently breaking hash-is-the-URL for exactly the content type most likely to
 * exceed it. The cap is on the OBJECT, not the payload, because the object is what is stored and
 * addressed: a capped payload would still produce a 1 MiB + 2 object.
 */
export const MAX_OBJECT_BYTES = 1_048_576;

export class ContentTooLargeError extends RangeError {
  constructor(readonly objectBytes: number) {
    super(
      `stele/content: object is ${objectBytes} bytes, over the ${MAX_OBJECT_BYTES}-byte cap — ` +
        'past this the hash is no longer the CID. Re-encode or downscale before submitting.',
    );
    this.name = 'ContentTooLargeError';
  }
}

export class MalformedContentError extends Error {
  constructor(message: string) {
    super(`stele/content: ${message}`);
    this.name = 'MalformedContentError';
  }
}

export interface EncodedContent {
  /** The full object — this is what gets stored, and what the hash is taken over. */
  bytes: Uint8Array;
  /** `sha256(bytes)`, the value written on-chain as `page_hash`. */
  hash: ContentHash;
  /** The same hash expressed as an IPFS CIDv1 raw block — the default read address. */
  cid: string;
}

/**
 * Wraps a payload in the scheme header and hashes it.
 *
 * @param kind One of `ContentKind`, or any id at or above `SITE_DEFINED_KIND_MIN`.
 */
export function encodeContent(kind: number, payload: Uint8Array): EncodedContent {
  assertByte(kind, 'kind');
  if (kind === 0) {
    throw new MalformedContentError('kind 0 is reserved and never valid — pick a ContentKind');
  }

  const objectBytes = HEADER_BYTES + payload.length;
  if (objectBytes > MAX_OBJECT_BYTES) throw new ContentTooLargeError(objectBytes);

  const bytes = new Uint8Array(objectBytes);
  bytes[0] = SCHEME_VERSION;
  bytes[1] = kind;
  bytes.set(payload, HEADER_BYTES);

  const hash = sha256Hex(bytes);
  return { bytes, hash, cid: contentHashToCid(hash) };
}

const utf8 = new TextEncoder();

/** UTF-8 bytes, unsanitized and unescaped — see the module note on why that is correct here. */
export function encodeText(text: string): EncodedContent {
  return encodeContent(ContentKind.Text, utf8.encode(text));
}

/**
 * A link payload is JSON — `{"href": "…"}`, optionally with a `"label"`.
 *
 * The renderers in `@stele/react` parse JSON and read `href`/`label`, so this helper writes that
 * shape rather than a bare URL string. Scheme policy is deliberately NOT here: `javascript:` is
 * refused at RENDER, because the renderer is the security boundary and a second list of "safe"
 * schemes would drift from the one actually enforced.
 */
export function encodeLink(href: string, label?: string): EncodedContent {
  if (typeof href !== 'string' || href.trim() === '') {
    throw new MalformedContentError('a link needs an href');
  }
  // `JSON.stringify` drops an undefined value, so an omitted label produces `{"href":"…"}`.
  return encodeContent(ContentKind.Link, utf8.encode(JSON.stringify({ href, label })));
}

export function encodeImage(bytes: Uint8Array): EncodedContent {
  return encodeContent(ContentKind.Image, bytes);
}

/**
 * The wrapper's payload layout. Two bytes of length rather than one because 255 is inside the range
 * of URLs people paste, and rather than four because the object cap is 1 MiB.
 */
const LINKED_HREF_LENGTH_BYTES = 2;
const MAX_HREF_BYTES = 0xffff;

/** Kinds that already carry a destination of their own, and therefore may never be wrapped. */
const NEVER_WRAPPED: readonly number[] = [ContentKind.Text, ContentKind.Link, ContentKind.Linked];

export interface DecodedLinked {
  /** The kind of the body — never `Linked`, never `Text`, never `Link`. */
  innerKind: number;
  /** Where it points, exactly as written. Scheme policy is the renderer's, not this function's. */
  href: string;
  /** The body, to be rendered as `innerKind` says. */
  body: Uint8Array;
}

/**
 * Wraps a body of another kind with a destination.
 *
 *     payload = [innerKind:u8][hrefLen:u16 big-endian][href utf8][body…]
 *
 * Nesting is refused rather than supported: one link is a decoration, two is a malformed object,
 * and a decoder that recurses on bytes somebody else wrote is a decoder with a depth limit to argue
 * about.
 */
export function encodeLinked(innerKind: number, href: string, body: Uint8Array): EncodedContent {
  assertByte(innerKind, 'innerKind');
  if (NEVER_WRAPPED.includes(innerKind)) {
    throw new MalformedContentError(
      `kind ${innerKind} already carries a destination — a text slot with a link is ContentKind.Link`,
    );
  }
  if (typeof href !== 'string' || href.trim() === '') {
    throw new MalformedContentError('a linked object needs an href');
  }

  const hrefBytes = utf8.encode(href);
  if (hrefBytes.length > MAX_HREF_BYTES) {
    throw new MalformedContentError(`href is ${hrefBytes.length} bytes, over the ${MAX_HREF_BYTES}-byte field`);
  }

  const payload = new Uint8Array(1 + LINKED_HREF_LENGTH_BYTES + hrefBytes.length + body.length);
  payload[0] = innerKind;
  payload[1] = (hrefBytes.length >>> 8) & 0xff;
  payload[2] = hrefBytes.length & 0xff;
  payload.set(hrefBytes, 3);
  payload.set(body, 3 + hrefBytes.length);

  return encodeContent(ContentKind.Linked, payload);
}

/**
 * Reads a `Linked` payload back. Throws on anything a renderer should treat as fallback — a
 * truncated header, a length that runs past the end, a body kind that may not be wrapped, or a
 * href that is not UTF-8.
 *
 * The length is checked against the payload it actually has rather than trusted: the bytes decide,
 * never the field claiming to describe them.
 */
export function decodeLinked(payload: Uint8Array): DecodedLinked {
  const header = 1 + LINKED_HREF_LENGTH_BYTES;
  if (payload.length < header) {
    throw new MalformedContentError(
      `linked payload is ${payload.length} bytes, shorter than its 3-byte header`,
    );
  }

  const innerKind = payload[0]!;
  if (innerKind === 0) throw new MalformedContentError('kind 0 is reserved and never valid');
  if (NEVER_WRAPPED.includes(innerKind)) {
    throw new MalformedContentError(`kind ${innerKind} may not be wrapped — see encodeLinked`);
  }

  const hrefLength = (payload[1]! << 8) | payload[2]!;
  const bodyStart = header + hrefLength;
  if (bodyStart > payload.length) {
    throw new MalformedContentError(
      `href claims ${hrefLength} bytes, past the end of a ${payload.length}-byte payload`,
    );
  }

  // `fatal`, so invalid UTF-8 throws instead of arriving as replacement characters in a URL.
  let href: string;
  try {
    href = new TextDecoder('utf-8', { fatal: true }).decode(payload.subarray(header, bodyStart));
  } catch {
    throw new MalformedContentError('href is not valid UTF-8');
  }
  if (href === '') throw new MalformedContentError('a linked object needs an href');

  return { innerKind, href, body: payload.subarray(bodyStart) };
}

export interface DecodedContent {
  schemeVersion: number;
  kind: number;
  payload: Uint8Array;
}

/**
 * Splits an object into its header and payload. Does NOT verify the hash — use `readContent` for
 * anything that will be rendered.
 */
export function decodeContent(bytes: Uint8Array): DecodedContent {
  if (bytes.length < HEADER_BYTES) {
    throw new MalformedContentError(`object is ${bytes.length} bytes, shorter than the 2-byte header`);
  }
  return {
    schemeVersion: bytes[0]!,
    kind: bytes[1]!,
    payload: bytes.subarray(HEADER_BYTES),
  };
}

export type ContentFailure =
  /** The bytes do not hash to what the chain says. Never render these. */
  | 'hash-mismatch'
  /** Shorter than the header, or a kind of 0. */
  | 'malformed'
  /** Verified, but written under a scheme this package does not know how to read. */
  | 'unknown-version';

export type ContentResult =
  | ({ ok: true } & DecodedContent)
  | { ok: false; reason: ContentFailure; schemeVersion?: number };

/**
 * The one function a renderer should call. Verify, then decode, in that order and never the other
 * way round.
 *
 * Nothing guarantees the bytes stay available: the chain will happily say a slot is owned and priced
 * while its content 404s. So every failure here is a `fallback`, and unverified bytes never reach a
 * render path — a hash mismatch is indistinguishable from an attacker substituting content, and has
 * to be treated as one.
 *
 * `unknown-version` is separated from `malformed` on purpose: it is the difference between "this
 * gateway served you garbage" and "this slot was written by someone on a newer SDK than you", and
 * only the second one is fixed by upgrading.
 */
export function readContent(bytes: Uint8Array, expectedHash: ContentHash): ContentResult {
  if (sha256Hex(bytes) !== expectedHash.toLowerCase()) {
    return { ok: false, reason: 'hash-mismatch' };
  }

  let decoded: DecodedContent;
  try {
    decoded = decodeContent(bytes);
  } catch {
    return { ok: false, reason: 'malformed' };
  }

  if (decoded.kind === 0) return { ok: false, reason: 'malformed' };
  if (decoded.schemeVersion !== SCHEME_VERSION) {
    return { ok: false, reason: 'unknown-version', schemeVersion: decoded.schemeVersion };
  }

  return { ok: true, ...decoded };
}

// ---------------------------------------------------------------------------
// Hashes
// ---------------------------------------------------------------------------

const HEX_64 = /^[0-9a-f]{64}$/;

/** Rejects anything that is not a 64-character lowercase hex digest. */
export function assertContentHash(hash: string): void {
  if (!HEX_64.test(hash)) {
    throw new MalformedContentError(`"${hash}" is not a 64-character lowercase hex sha256 digest`);
  }
}

/** `sha256(bytes)` as the 64-character lowercase hex the site stores in `page_hash`. */
export function sha256Hex(bytes: Uint8Array): ContentHash {
  return bytesToHex(sha256(bytes));
}

const BASE64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

/**
 * `sha256(bytes)` in base64 — the NEP-177 `media_hash`, required whenever `media` is set.
 *
 * Not interchangeable with `sha256Hex`: a CID is a multihash (codec and length prefix included)
 * while `media_hash` is the bare digest, and the two encodings are the two different questions the
 * two fields ask.
 */
export function mediaHash(bytes: Uint8Array): string {
  return base64Encode(sha256(bytes));
}

// ---------------------------------------------------------------------------
// CID — the hash IS the address
// ---------------------------------------------------------------------------

/** RFC 4648 base32, lowercase, no padding — multibase `b`. */
const BASE32 = 'abcdefghijklmnopqrstuvwxyz234567';

/**
 * `[cidVersion=1][multicodec=raw][multihash=sha2-256][length=32]`, the prefix that makes every
 * stele CID start `bafkrei`.
 */
const CID_PREFIX = Uint8Array.from([0x01, 0x55, 0x12, 0x20]);

/**
 * Renders a content hash as the IPFS CIDv1 raw block that addresses the same bytes.
 *
 * This is why sha256 was chosen: a client with nothing but the on-chain hash can construct a
 * gateway URL. No storage adapter, no mapping table, no backend on the read path.
 */
export function contentHashToCid(hash: ContentHash): string {
  const digest = hexToBytes(hash);
  if (digest.length !== 32) {
    throw new MalformedContentError(`content hash must be 32 bytes, got ${digest.length}`);
  }
  const cid = new Uint8Array(CID_PREFIX.length + 32);
  cid.set(CID_PREFIX, 0);
  cid.set(digest, CID_PREFIX.length);
  return `b${base32Encode(cid)}`;
}

/**
 * The inverse. Useful for checking that a gateway URL a site has stored actually addresses the hash
 * the chain holds — when the two drift, the slot renders someone else's content with no error.
 */
export function cidToContentHash(cid: string): ContentHash {
  if (!cid.startsWith('b')) {
    throw new MalformedContentError(`expected a base32 multibase CID starting "b", got "${cid.slice(0, 8)}"`);
  }
  const bytes = base32Decode(cid.slice(1));
  if (bytes.length < CID_PREFIX.length + 32) {
    throw new MalformedContentError(`CID decodes to ${bytes.length} bytes, too short for a raw sha256 block`);
  }
  for (let i = 0; i < CID_PREFIX.length; i++) {
    if (bytes[i] !== CID_PREFIX[i]) {
      throw new MalformedContentError('CID is not a CIDv1 raw block with a sha2-256 multihash');
    }
  }
  return bytesToHex(bytes.subarray(CID_PREFIX.length, CID_PREFIX.length + 32));
}

// ---------------------------------------------------------------------------
// IPFS — `media` is a URI, not a digest
// ---------------------------------------------------------------------------

/** The `media` field of a NEP-177 token: `ipfs://<CID>`. */
export function ipfsUri(cid: string): string {
  if (cid.trim() === '') throw new MalformedContentError('an IPFS URI needs a CID');
  return `ipfs://${cid}`;
}

/** The CID out of an `ipfs://` URI. Refuses any other scheme rather than guessing what it holds. */
export function cidFromIpfsUri(uri: string): string {
  const prefix = 'ipfs://';
  if (!uri.startsWith(prefix)) {
    throw new MalformedContentError(`"${uri}" is not an ipfs:// URI`);
  }
  const cid = uri.slice(prefix.length);
  if (cid === '') throw new MalformedContentError(`"${uri}" carries no CID`);
  return cid;
}

/** `https://<gateway>/<cid>` with exactly one slash between the parts. */
export function ipfsGatewayUrl(cid: string, gateway: string): string {
  if (cid.trim() === '') throw new MalformedContentError('a gateway URL needs a CID');
  return `${gateway.replace(/\/+$/, '')}/${cid}`;
}

// ---------------------------------------------------------------------------
// Encoders
// ---------------------------------------------------------------------------

function base64Encode(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i]!;
    const b1 = bytes[i + 1];
    const b2 = bytes[i + 2];
    out += BASE64[b0 >>> 2]!;
    out += BASE64[((b0 & 0x03) << 4) | ((b1 ?? 0) >>> 4)]!;
    out += b1 === undefined ? '=' : BASE64[((b1 & 0x0f) << 2) | ((b2 ?? 0) >>> 6)]!;
    out += b2 === undefined ? '=' : BASE64[b2 & 0x3f]!;
  }
  return out;
}

function base32Encode(bytes: Uint8Array): string {
  let out = '';
  let value = 0;
  let bits = 0;
  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      out += BASE32[(value >>> (bits - 5)) & 31]!;
      bits -= 5;
    }
  }
  if (bits > 0) out += BASE32[(value << (5 - bits)) & 31]!;
  return out;
}

function base32Decode(text: string): Uint8Array {
  const out: number[] = [];
  let value = 0;
  let bits = 0;
  for (const char of text) {
    const index = BASE32.indexOf(char);
    if (index < 0) throw new MalformedContentError(`"${char}" is not a base32 character`);
    value = (value << 5) | index;
    bits += 5;
    if (bits >= 8) {
      out.push((value >>> (bits - 8)) & 0xff);
      bits -= 8;
    }
  }
  return Uint8Array.from(out);
}

function hexToBytes(hex: ContentHash): Uint8Array {
  if (hex.length % 2 !== 0) throw new MalformedContentError('hex string has an odd length');
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    const byte = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    if (Number.isNaN(byte)) throw new MalformedContentError(`"${hex}" is not valid hex`);
    out[i] = byte;
  }
  return out;
}

function bytesToHex(bytes: Uint8Array): string {
  let out = '';
  for (const byte of bytes) out += byte.toString(16).padStart(2, '0');
  return out;
}

function assertByte(value: number, name: string): void {
  if (!Number.isInteger(value) || value < 0 || value > 255) {
    throw new MalformedContentError(`${name} must be a u8, got ${value}`);
  }
}
