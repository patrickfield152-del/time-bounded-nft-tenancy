/**
 * Content addressing: payload framing (`u8` `kind`, `String` `page_hash`, `mediaHash`, `sha256Hex`, `contentHashToCid`, `encumbrance_hash`) and `ipfs://<CID>`.
 * Renderer escapes untrusted content at render time.
 */
import { sha256 } from '@noble/hashes/sha2.js';
import { bytesToHex, hexToBytes as nobleHexToBytes } from '@noble/hashes/utils.js';

/** A SHA-256 digest as 64 lowercase hex characters. The contract stores this form directly. */
export type ContentHash = string;

/** Preimage scheme version; bump for new object layouts. */
export const SCHEME_VERSION = 1;

/** Payload kind ids (`0` is reserved; `SITE_DEFINED_KIND_MIN`+ are site-owned). */
export const ContentKind = {
  Text: 1,
  Link: 2,
  Image: 3,
  Video: 4,
  /** Wrapper attaching an href to another kind (see `encodeLinked`). */
  Linked: 5,
} as const;

export type ContentKind = (typeof ContentKind)[keyof typeof ContentKind];

/** Kind ids at or above this are reserved for the site and never interpreted by this package. */
export const SITE_DEFINED_KIND_MIN = 128;

/** The header is two bytes: `[schemeVersion][kind]`. */
export const HEADER_BYTES = 2;

/** Enforced at encode time (above 1 MiB, IPFS chunks into DAG-PB and sha256 != CID). */
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
  /** Framed bytes (`[schemeVersion][kind][payload]`). */
  bytes: Uint8Array;
  /** `sha256(bytes)` written on-chain as `page_hash`. */
  hash: ContentHash;
  /** IPFS CIDv1 raw block address. */
  cid: string;
}

/** Wraps `payload` with the scheme header (`ContentKind` or `>= SITE_DEFINED_KIND_MIN`) and hashes it. */
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

/** JSON link payload (`{"href": "…"}`, optional `"label"` / `label`); `@stele/react` parses it and filters `javascript:` at render. */
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

/** Wrapper header: `[innerKind:u8][hrefLen:u16 BE][href][body]`. */
const HREF_LEN_B = 2;
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

/** Wraps `body` with an `href` destination; nested links are rejected. */
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

  const payload = new Uint8Array(1 + HREF_LEN_B + hrefBytes.length + body.length);
  payload[0] = innerKind;
  payload[1] = (hrefBytes.length >>> 8) & 0xff;
  payload[2] = hrefBytes.length & 0xff;
  payload.set(hrefBytes, 3);
  payload.set(body, 3 + hrefBytes.length);

  return encodeContent(ContentKind.Linked, payload);
}

/** Decodes a `Linked` payload; throws on truncated, invalid-kind, or non-UTF-8 inputs. */
const fatalUtf8 = new TextDecoder('utf-8', { fatal: true });

export function decodeLinked(payload: Uint8Array): DecodedLinked {
  const header = 1 + HREF_LEN_B;
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
    href = fatalUtf8.decode(payload.subarray(header, bodyStart));
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

/** Splits framed object into header and payload without verifying the hash (renderers use `readContent`). */
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

/** Verifies sha256 before decoding; renderers degrade `malformed` / `unknown-version` / mismatch to `fallback`. */
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

// Hashes

const HEX_64 = /^[0-9a-f]{64}$/;

/** Rejects anything that is not a 64-character lowercase hex digest. */
export function assertContentHash(hash: string): void {
  if (!HEX_64.test(hash)) {
    throw new MalformedContentError(`"${hash}" is not a 64-character lowercase hex sha256 digest`);
  }
}

/** `sha256(bytes)` as the 64-character lowercase hex the site stores in `page_hash`. */
const NATIVE_TO_HEX = typeof Uint8Array.prototype.toHex === 'function';
const NATIVE_FROM_HEX = typeof Uint8Array.fromHex === 'function';
const NATIVE_TO_BASE64 = typeof Uint8Array.prototype.toBase64 === 'function';

function toHex(bytes: Uint8Array): string {
  return NATIVE_TO_HEX ? bytes.toHex() : bytesToHex(bytes);
}

/** `sha256(bytes)` as 64 lowercase hex (`page_hash`). */
export function sha256Hex(bytes: Uint8Array): ContentHash {
  return toHex(sha256(bytes));
}

/** `sha256(bytes)` in base64 (`media_hash`, distinct from multihash CID). */
export function mediaHash(bytes: Uint8Array): string {
  return base64Encode(sha256(bytes));
}

// CID — the hash IS the address

/** RFC 4648 base32, lowercase, no padding (`b` multibase; `[cidVersion=1][multicodec=raw][multihash=sha2-256][length=32]`). */
const BASE32 = 'abcdefghijklmnopqrstuvwxyz234567';
const CID_PREFIX = Uint8Array.from([0x01, 0x55, 0x12, 0x20]);
const BASE32_CODES = Uint8Array.from([...BASE32].map((c) => c.charCodeAt(0)));
const BASE32_LUT = new Int8Array(128).fill(-1);
for (let i = 0; i < 32; i++) BASE32_LUT[BASE32.charCodeAt(i)] = i;

// `CID_PREFIX` (32 bits) encodes to `bafkrei` + 2 zero carry bits, so only the 32 digest bytes vary.
const CID_OUT = new Uint8Array(59);
for (let i = 0; i < 7; i++) CID_OUT[i] = 'bafkrei'.charCodeAt(i);
const CID_DEC = new Uint8Array(32);

/** Formats `hash` as its IPFS CIDv1 raw block (`bafkrei…`). */
export function contentHashToCid(hash: ContentHash): string {
  const d = hexToBytes(hash);
  if (d.length !== 32) {
    throw new MalformedContentError(`content hash must be 32 bytes, got ${d.length}`);
  }
  let n = 7, v = 0, b = 2;
  for (let i = 0; i < 32; i++) {
    v = (v << 8) | d[i]!;
    b += 8;
    while (b >= 5) {
      CID_OUT[n++] = BASE32_CODES[(v >>> (b - 5)) & 31]!;
      b -= 5;
    }
  }
  if (b > 0) CID_OUT[n++] = BASE32_CODES[(v << (5 - b)) & 31]!;
  return String.fromCharCode.apply(null, CID_OUT as unknown as number[]);
}

/** Extracts the 32-byte sha256 hex digest from a CIDv1 raw multibase string. */
export function cidToContentHash(cid: string): ContentHash {
  if (!cid.startsWith('b')) {
    throw new MalformedContentError(`expected a base32 multibase CID starting "b", got "${cid.slice(0, 8)}"`);
  }
  const raw = cid.slice(1);
  const byteLen = Math.floor((raw.length * 5) / 8);
  if (byteLen < CID_PREFIX.length + 32) {
    throw new MalformedContentError(`CID decodes to ${byteLen} bytes, too short for a raw sha256 block`);
  }
  let n = 0, v = 0, b = 0;
  for (let i = 0; i < raw.length; i++) {
    const code = raw.charCodeAt(i);
    const idx = code < 128 ? BASE32_LUT[code]! : -1;
    if (idx < 0) throw new MalformedContentError(`"${raw[i]}" is not a base32 character`);
    v = (v << 5) | idx;
    b += 5;
    if (b >= 8) {
      const byte = (v >>> (b - 8)) & 0xff;
      if (n < CID_PREFIX.length) {
        if (byte !== CID_PREFIX[n]) {
          throw new MalformedContentError('CID is not a CIDv1 raw block with a sha2-256 multihash');
        }
      } else if (n < CID_PREFIX.length + 32) {
        CID_DEC[n - CID_PREFIX.length] = byte;
      }
      n++;
      b -= 8;
    }
  }
  return toHex(CID_DEC);
}

// IPFS — `media` is a URI, not a digest

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

// Encoders
/** Native `toBase64` (Node 25+, V8 13.6), `Buffer` (255, 25, 13.6), or `btoa` (`as` needed for global `Buffer` check). */
function base64Encode(bytes: Uint8Array): string {
  if (NATIVE_TO_BASE64) return bytes.toBase64();
  // `Buffer` is a Node global absent from the DOM lib.
  const BufferCtor = (
    globalThis as unknown as {
      Buffer?: { from(input: Uint8Array): { toString(encoding: string): string } };
    }
  ).Buffer;
  if (BufferCtor) return BufferCtor.from(bytes).toString('base64');
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function hexToBytes(hex: ContentHash): Uint8Array {
  if (hex.length % 2 !== 0) throw new MalformedContentError('hex string has an odd length');
  try {
    return NATIVE_FROM_HEX ? Uint8Array.fromHex(hex) : nobleHexToBytes(hex);
  } catch {
    throw new MalformedContentError(`"${hex}" is not valid hex`);
  }
}



function assertByte(value: number, name: string): void {
  if (!Number.isInteger(value) || value < 0 || value > 255) {
    throw new MalformedContentError(`${name} must be a u8, got ${value}`);
  }
}
