# stele

**NEP-171 slot NFTs + time-bounded function-call-key tenancy, NEP-141 settlement.**

A publisher divides a page into named slots; each slot is a NEP-171 token whose holder may write the
content behind that slot, sell it at the site's own mean-reverting price curve, or rent it out for a
fixed term. Tenancy is a time-bounded function-call access key: the tenant signs content writes with
a session key scoped to the site contract and to `edit`, while the contract asserts the rental
window on every gated call, because a function-call key carries no expiry of its own. Settlement is
NEP-141 (native NEAR on the native branch), payouts go through a pull ledger, and every site is its
own account with its own storage and its own frozen code.

## Status

| Wave | Scope | State |
|---|---|---|
| W1 REPO | repository scaffold, toolchain pins, state/type layouts | done |
| W2 PRIMITIVE | parity vectors, pricing/rentals/terms, site NFT core, SDK/React | done |
| W3 FLOWS | tenancy/edit/claim, factory, escrow, mock FT, sandbox harness | done, descoped (below) |
| W4 CLOSE | README, numbers, sweep, clean-room | done |

### W3 descope (still open)

- **Escrowed-site XCC gas.** `create_escrowed_site` fails inside its vault/site XCC chain at receipt
  `i=3` with `Exceeded the prepaid gas` (12.808 TGas); the `+0.89 TGas` is the fixed
  function-call/receipt-creation overhead, so the fix is to attach at `allotment - 1 Tgas`, not to
  raise caps.
- **Timestamp-based rental expiry.** `sandbox_fast_forward` advances height only; block timestamps do
  not move, so the post-expiry edit and `end_rental`/hash-restore assertions are not reachable in the
  harness. The fix is to patch `Rental.expiry`/`BlockInfo` instead of advancing height.
- The descoped dry-run (`STELE_SKIP_ESCROW=1 STELE_SKIP_EXPIRY=1`) finishes `DRY-RUN OK`.

## Quickstart

```bash
# 1. contracts (rustc 1.100.0, pinned by rust-toolchain.toml)
cargo check --workspace --target wasm32-unknown-unknown
cargo test -p stele-site --tests            # 10 parity + 26 terms + 2 views = 38

# 2. wasm artifacts
for c in site factory escrow mock-ft; do (cd contracts/$c && cargo near build non-reproducible-wasm); done

# 3. client (pnpm 12.18.0, Node >= 26.13.1)
pnpm install
pnpm -C packages/sdk typecheck
pnpm -C packages/react typecheck
pnpm -C packages/sdk build
pnpm -C packages/react build
pnpm exec vitest run packages/sdk/src

# 4. zero-funds dry-run (Linux/macOS; WSL2 on Windows)
STELE_SKIP_ESCROW=1 STELE_SKIP_EXPIRY=1 \
STELE_SITE_WASM=$PWD/target/near/stele_site/stele_site.wasm \
STELE_FACTORY_WASM=$PWD/target/near/stele_factory/stele_factory.wasm \
STELE_ESCROW_WASM=$PWD/target/near/stele_escrow/stele_escrow.wasm \
STELE_MOCK_FT_WASM=$PWD/target/near/stele_mock_ft/stele_mock_ft.wasm \
node scripts/sandbox-dry-run.mjs
```

## Numbers

- Parity vectors: **3,024** take-price rows + **690** v2 ask/rent/fee rows, read from
  `vectors/manifest.json` (sha256 per file). Reproduce: `cargo test -p stele-site --test parity` —
  10 tests, 0 failed.
- Terms/ratchet unit tests: **26**, 0 failed (`cargo test -p stele-site --test terms`).
- View/encumbrance tests: **2**, 0 failed (`cargo test -p stele-site --test views`).
- SDK behavior suite: **15** tests in one file (`packages/sdk/src/stele.test.ts`), 0 failed
  (`pnpm exec vitest run packages/sdk/src`).
- Measured claim: **3,714 parity vector executions** (3,024 + 690), 38 Rust + 15 SDK tests,
  **0 failed**. Gas is not claimed as a headline; the sandbox can print per-receipt `gas_burnt` if a
  later pass needs it.
- Release wasm sizes (`cargo near build non-reproducible-wasm`, ABI embedded, `opt-level = "z"`,
  `cdylib`-only on factory/escrow/mock-ft): site 428,124 B; factory 183,047 B; escrow 179,537 B;
  mock-ft 125,535 B.

## Layout

```
near-stele/
|-- contracts/{site,factory,escrow,mock-ft}
|-- vectors/{price-vectors.jsonl,v2-vectors.jsonl,encumbrance-vectors.jsonl,manifest.json}
|-- packages/{sdk,react}
|-- apps/demo
|-- scripts/{regen-vectors-from-origin.sh,sandbox-dry-run.mjs}
|-- tests/sandbox
|-- rust-toolchain.toml, .nvmrc, pnpm-workspace.yaml
```

## Toolchain pins

Rust 1.100.0 (edition 2024, `rust-toolchain.toml`) · near-sdk 5.30.2 ·
near-sdk-contract-tools 4.0.0 · primitive-types 0.14.0 · near-api-js 7.5.3 ·
near-sandbox 0.3.5 · @hot-labs/near-connect 0.12.0 · near-connect-hooks 1.1.6 ·
typescript 7.0.2 · tsx 4.23.15 · tsup 9.0.0 · vitest 5.1.0 · next 16.4.6 ·
react/react-dom 19.3.1 · @types/node 26.6.6 · Node >= 26.13.1 (`.nvmrc`) ·
pnpm 12.18.0 · cargo-near 0.23.0 · wasm-opt 0.116.1.

`pnpm-workspace.yaml` holds `autoDedupe`, `dedupePeers`, and `overrides` keeping exactly one
`@hot-labs/near-connect` version in the lockfile. Lockfiles: `Cargo.lock` + `pnpm-lock.yaml`.

## Environment

`.env.example` carries `NEAR_FACTORY_ACCOUNT`, `NEAR_SITE_ACCOUNT`, `NEAR_VAULT_ACCOUNT`,
`NEAR_FT_ACCOUNT`, `NEAR_RPC_URL` (testnet example values, no keys).

## Limits

- Escrowed-site creation and timestamp-based expiry tests are descoped (see above).
- The sandbox is Unix-only and runs zero-funds; no mainnet deployment is part of this repository.
- The site's `min_slot_floor` is supplied by the factory from the settlement asset's real decimals
  (native 24 ⇒ `1e20`; 6-decimal mock FT ⇒ `100`); it is never derived from `ft_metadata` inside
  `new`, because that call is asynchronous.
- Royalty info is display-only; `buy` never deducts it.

## Origin and license

Port of an MIT-licensed EVM product; provenance, baseline commit and archive hash are in
`ORIGIN.md`, and the upstream notice is reproduced in `ATTRIBUTION.md`. MIT license kept. There is no
git remote; publishing is a human action.

## Hackathon

Track: `nfts`. Development ends 2026-11-14T23:59:00Z (UTC).
