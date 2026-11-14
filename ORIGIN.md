# Origin

`stele` is a port of an existing EVM project. This file is the provenance record: what was imported,
at which commit, and what the port replaces. The imported tree is the whole starting point of the
repository; there is no second upstream.

## Baseline

| Field | Value |
|---|---|
| Origin repository | https://github.com/vibecodermaxi/websitekit |
| Origin branch | `main` |
| Origin HEAD | `82ff6db8d7239475aafbc5aac23175907583fa80` |
| Origin archive | `https://codeload.github.com/vibecodermaxi/websitekit/tar.gz/82ff6db8d7239475aafbc5aac23175907583fa80` |
| Archive sha256 | `28b0132e1b27197d02df54b97e76a9e09c520d128ba212ff89ec8ac257839f10` |
| Local baseline commit | `30779ef151801b79a4385ebe92c9db6c53dff3e3` |
| Remote baseline commit | `61c45d868ab6190c57bbbe0959046f7de3be952c` |
| Remote build box | `maxwell-fr9jn` (`cmp_e72322cece72`), checkout at `~/work/stele` |
| License | MIT, kept; the origin notice is reproduced in `ATTRIBUTION.md` and both notices are in `LICENSE` |

The archive above was imported as one commit on branch `main`; that import is the local baseline
commit in the table. The remote baseline commit is the same tree as it exists on the build box, which
is where builds and test runs happen.

## What the port replaces

The origin is an EVM product: Solidity 0.8.30 contracts built with Foundry (ERC-721 slot tokens, an
inlined pricing library with a weekly mean-reverting decay loop, a rental state machine, a clone
factory and an escrowed vault), a TypeScript SDK built on `viem`, React components, and a Next.js
docs site. The port keeps the mechanism and changes the platform: the contracts become Rust NEAR
contracts on NEP-171/177/178/181 for the slot tokens and NEP-141 for settlement, with per-site
sub-accounts and on-chain storage staking replacing EVM clones and balance storage; the tenancy
becomes a time-bounded function-call access key enforced by the contract, replacing ERC-4907
user-expiry; and the SDK is rebuilt on `near-api-js` 7.5.3, with `viem` dropped entirely. The two
pure-math SDK modules carry the origin's arithmetic as parity twins, pinned to the origin contracts
by the generated vectors; everything that touches chain transport is rewritten.

## Remote and publishing

This repository has no git remote, by policy. Publishing is a human action: a human adds the remote,
pushes, and makes the repository public. Nothing in the tree pushes anywhere, and no script in the
tree assumes a remote exists.

## Reproducing the parity vectors

`scripts/regen-vectors-from-origin.sh` clones the origin at the commit above into a temporary
directory, runs the two Foundry vector generators inside `packages/websitekit-contracts`, copies the
resulting `price-vectors.jsonl` and `v2-vectors.jsonl` into `vectors/`, and writes
`vectors/manifest.json` with the row count and sha256 of each file plus the `forge` and `solc`
versions that produced them. The vectors are generated once and committed; the script is not part of
any build or test path.
