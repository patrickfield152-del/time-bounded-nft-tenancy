# Attribution

## Origin notice
The upstream project is MIT licensed. This is its copyright notice as published:

```
MIT License

Copyright (c) 2026 websitekit
```

`LICENSE` holds the full permissive terms. It carries that notice and the port's own copyright line.

## Files carried over verbatim
Two SDK modules cross the platform boundary untouched.

- `packages/sdk/src/pricing.ts`
- `packages/sdk/src/rentals.ts`

Both files are byte-identical to the origin SDK and stay parity twins. The port never edits either file. The Rust implementations are checked against the same generated vectors. Their arithmetic is the parity target, so a change to either file is a parity break, not a refactor.

## Not carried over
The origin vendored three Solidity libraries: forge-std, OpenZeppelin Contracts, and solady. They were build dependencies of the EVM contracts, not port surface. The port did not carry them over. No Solidity source is part of this repository.

## Dependency licenses
- `near-sdk` 5.30.1 and `primitive-types` 0.14.0: MIT OR Apache-2.0.
- `near-sdk-contract-tools` 4.0.0: GPL-3.0 OR Apache-2.0. This port uses the Apache-2.0 option and does not incorporate or link the GPL-3.0 branch.
