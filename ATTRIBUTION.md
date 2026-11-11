# Attribution

## Origin notice

The upstream project is MIT licensed. Its copyright notice, reproduced as published:

```
MIT License

Copyright (c) 2026 websitekit
```

The full permissive terms are in `LICENSE`, which carries that notice together with the port's own
copyright line.

## Files carried over verbatim

Two SDK modules cross the platform boundary untouched:

- `packages/sdk/src/pricing.ts`
- `packages/sdk/src/rentals.ts`

They are carried over from the origin SDK byte-identically and are kept as parity twins: the port
does not edit either file, and the Rust implementations are checked against the same generated
vectors as these modules. Their arithmetic is the parity target, so a change to either file is a
parity break rather than a refactor.

## Not carried over

The origin's vendored Solidity libraries (forge-std, OpenZeppelin Contracts, solady) were build
dependencies of the EVM contracts and are not port surface. They were not carried over, and no
Solidity source is part of this repository.

## Dependency licenses

- `near-sdk` 5.30.1 and `primitive-types` 0.14.0: MIT OR Apache-2.0.
- `near-sdk-contract-tools` 4.0.0: GPL-3.0 OR Apache-2.0; this port uses the Apache-2.0 option and
  does not incorporate or link the GPL-3.0 branch.
