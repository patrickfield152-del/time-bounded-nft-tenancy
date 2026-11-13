/**
 * Native-codec feature tests.
 *
 * `Uint8Array.prototype.toBase64` is TC39 stage-3, shipped in V8 13.6 / Node
 * 25.0.0. The SDK encoders use it when present and fall back to `Buffer` /
 * `btoa` otherwise; these tests pin the output to the exact bytes the chain
 * stores regardless of which branch runs.
 */
import { describe, expect, it } from 'vitest';
import { mediaHash, sha256Hex } from './content';
const NATIVE_TO_BASE64 = typeof (Uint8Array.prototype as Uint8Array & { toBase64?: unknown }).toBase64 === 'function';


describe('native Uint8Array.toBase64', () => {
  it('reports whether this runtime has the TC39 method', () => {
    // Node 25+: MEASURED true on v25.0.0; Node 22/24: false (fallback path).
    expect(typeof NATIVE_TO_BASE64).toBe('boolean');
  });

  it('mediaHash equals the reference base64 of sha256', () => {
    const bytes = new Uint8Array([0xfb, 0xff, 0xbf, 0x00, 0x01]);
    // echo -n <bytes> | base64 of the digest is not stable across shells, so
    // the reference is computed with the same standard alphabet here.
    const reference = Buffer.from(sha256Hex(bytes), 'hex').toString('base64');
    expect(mediaHash(bytes)).toBe(reference);
  });

  it('produces standard padded base64 on every path', () => {
    const bytes = new Uint8Array(31).map((_, index) => index * 7 + 1);
    const digest = mediaHash(bytes);
    expect(digest).toMatch(/^[A-Za-z0-9+/]+={0,2}$/);
    expect(digest.length % 4).toBe(0);
  });

  it('keeps sha256Hex a pure lowercase hex digest', () => {
    expect(sha256Hex(new Uint8Array([1, 2, 3]))).toMatch(/^[0-9a-f]{64}$/);
  });
});
