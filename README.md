# stele

**NEP-171 slot NFTs + time-bounded function-call-key tenancy, NEP-141 settlement.**

A publisher divides a page into named slots; each slot is a NEP-171 token whose holder may write the
content behind that slot, sell it at the site's own mean-reverting price curve, or rent it out for a
fixed term. Tenancy is a time-bounded function-call access key: the tenant signs content writes with
a session key scoped to the site contract and to its content-write method, while the contract asserts
the rental window on every gated call, because a function-call key carries no expiry of its own. All
settlement is NEP-141 (native NEAR on the native branch), payouts go through a pull ledger, and every
site is its own account with its own storage and its own frozen code.

## Status

| Wave | Scope | State |
|---|---|---|
| W1 REPO (gates G0 and G1) | repository scaffolding, toolchain pins, contract-crate workspace, state and type layouts | this commit; the only wave that exists |
| W2 PRIMITIVE | pricing, rentals and terms parity, plus the site NFT core (NEP-171/177/178/181) | not written |
| W3 FLOWS | market spine, escrow vault, mock NEP-141 settlement token, SDK, React package, demo app, sandbox dry-run | not written |
| W4 CLOSE | numbers pin, README-as-spec rewrite, legacy sweep | not written |

Only W1 is present. The contract crates in this tree define state and type layouts; no market,
tenancy, factory, escrow or token method exists yet, and the SDK, React package, demo app and
dry-run harness in the layout below are W3 targets.

## Target layout

```
near-stele/
|-- README.md                      # executable spec; names the primitive and its numbers
|-- LICENSE                        # MIT, with the origin attribution
|-- ORIGIN.md                      # provenance, baseline commit, diff link
|-- Cargo.toml                     # workspace over contracts/*
|-- package.json / pnpm-workspace.yaml
|-- contracts/
|   |-- site/                      # stele-site <- SlotSite + Pricing + RentalsLib + TermsLib
|   |   `-- src/{lib.rs, transfer_hook.rs, pricing.rs, rentals.rs, terms.rs, ft_receiver.rs, views.rs, events.rs}
|   |-- factory/                   # stele-factory <- SlotFactory + EscrowFactory + Attestor
|   |-- escrow/                    # stele-escrow <- EscrowVault
|   `-- mock-ft/                   # stele-mock-ft <- TestUSD (NEP-141/145/148)
|-- vectors/{price-vectors.jsonl, v2-vectors.jsonl, manifest.json}   # generated once, committed; manifest = rows + sha256 per file
|-- packages/
|   |-- sdk/                       # @stele/sdk: pricing/rentals reused; content/keys rewritten
|   |   `-- src/                   # near.ts, reads.ts, writes.ts, escrow.ts rewritten; tenancy.ts new
|   `-- react/                     # @stele/react <- Slot.tsx, BuyDialog.tsx, useSlot.ts, useBuy.ts, content-store.ts, context.tsx, format.ts, index.ts
|-- apps/demo/                     # one demo board page
|-- scripts/{regen-vectors-from-origin.sh, deploy-testnet.sh, sandbox-dry-run.mjs, measure-method-gas.mjs}
`-- tests/                         # near-sandbox integration
```

`.gitignore`, `.env.example` and `ATTRIBUTION.md` also sit at the repository root.

## Toolchain pins

| Component | Pin |
|---|---|
| Rust | >= 1.95, edition 2024 |
| near-sdk | 5.30.1 |
| near-sdk-contract-tools | 4.0.0 |
| primitive-types | 0.14.0 |
| cargo-near | 0.23.0 |
| near-cli-rs | 0.31.1 |
| Node.js | >= 22.22.2 |
| pnpm | >= 10.4.1 |
| near-api-js | 7.5.3 |
| near-sandbox | 0.3.5 |
| vitest | 4.1.10 |
| @hot-labs/near-connect | 0.11.x (hooks-bundled) |
| near-connect-hooks | 1.1.6 |

`@hot-labs/near-connect` stays on the 0.11.x line because `near-connect-hooks` 1.1.6 depends on
`^0.11.1`; a 0.12.0 root would install two copies of the connector. The sandbox harness ships Linux
and macOS binaries only, so on a Windows host the dry-run belongs inside WSL2.

## Build

The contract workspace is checked against the wasm target the crates are deployed as:

```bash
rustup target add wasm32-unknown-unknown
cargo check --workspace --target wasm32-unknown-unknown
```

## Environment

`.env.example` holds the five variables the deploy scripts and the SDK read. The values are example
testnet account names; the file carries no keys and no secrets.

| Variable | Purpose |
|---|---|
| `NEAR_FACTORY_ACCOUNT` | factory account, which also holds the attestor registry |
| `NEAR_SITE_ACCOUNT` | demo site, a direct sub-account of the factory |
| `NEAR_VAULT_ACCOUNT` | demo escrow vault, a direct sub-account of the factory |
| `NEAR_FT_ACCOUNT` | mock NEP-141 settlement token |
| `NEAR_RPC_URL` | testnet RPC endpoint |

## Not yet implemented

The contract crates contain state and type layouts only. Every method lands in W2 and W3: the pricing
and rental math, the slot NFT core, the market spine, the factory, the escrow vault and the mock
NEP-141 token. Nothing here deploys, mints, prices, rents or settles, and this file claims no test
result, dry-run result or measurement.

## License and provenance

MIT. `LICENSE` carries the two copyright lines. `ORIGIN.md` records the upstream repository, branch,
commit and archive hashes, and states what the port replaces. `ATTRIBUTION.md` reproduces the
upstream copyright notice and names the SDK files that are carried over byte-identically.
