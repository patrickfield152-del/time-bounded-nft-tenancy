# Attribution

## Origin notice
The upstream project is MIT licensed. This is its copyright notice as published:

```
MIT License

Copyright (c) 2026 websitekit
```

`LICENSE` holds the full permissive terms. It carries that notice and the port's own copyright line.
The upstream repository is `https://github.com/vibecodermaxi/websitekit`, branch `main`, HEAD
`82ff6db8d7239475aafbc5aac23175907583fa80`. [`ORIGIN.md`](ORIGIN.md) holds the archive URL and its sha256.

## Files carried over as parity twins
Two SDK modules cross the platform boundary with their arithmetic intact.

- `packages/sdk/src/pricing.ts`
- `packages/sdk/src/rentals.ts`

Both files restate the origin's parity-twin TypeScript in the port's documented form: the upstream
copies under `packages/websitekit-sdk/src/` remain the origin's, while these are re-commented and
slightly restructured for this port. The origin arithmetic is the parity target: every price/rent
path is checked against the same generated vectors (3,714 rows), so a change to the arithmetic in
either file is a parity break, not a refactor.

## Not carried over
The origin vendored three Solidity libraries: forge-std, OpenZeppelin Contracts, and solady. They were build dependencies of the EVM contracts, not port surface. The port did not carry them over. No Solidity source is part of this repository.

## Dependency licenses
- `near-sdk` 5.30.2 and `primitive-types` 0.14.0: MIT OR Apache-2.0.
- `near-sdk-contract-tools` 4.0.0: GPL-3.0 OR Apache-2.0. This port uses the Apache-2.0 option and does not incorporate or link the GPL-3.0 branch.
