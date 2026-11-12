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
| W4 CLOSE | README, numbers, sweep, clean-room | in progress |

### W3 descope (documented, W4 items)

- **Escrowed-site XCC gas.** `create_escrowed_site` fails inside its vault/site XCC chain at receipt
  `i=3` with `Exceeded the prepaid gas` (12.808 TGas); the `+0.89 TGas` is the fixed
  function-call/receipt-creation overhead, so the fix is to attach at `allotment - 1 Tgas`, not to
  raise caps.
- **Timestamp-based rental expiry.** `sandbox_fast_forward` advances height only; block timestamps do
  not move, so the post-expiry edit and `end_rental`/hash-restore assertions are not reachable in the
  harness. W4 fixes this by patching `Rental.expiry`/`BlockInfo` instead of advancing height.
- The descoped dry-run (`STELE_SKIP_ESCROW=1 STELE_SKIP_EXPIRY=1`) finishes `DRY-RUN OK`.

## Quickstart

```bash
# 1. contracts
cargo check --workspace --target wasm32-unknown-unknown
cargo test -p stele-site --tests            # 10 parity + 26 terms

# 2. wasm artifacts
for c in site factory escrow mock-ft; do (cd contracts/$c && cargo near build non-reproducible-wasm); done

# 3. client
corepack pnpm install
pnpm -C packages/sdk typecheck
pnpm -C packages/react typecheck

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
- wasm sizes (release, ABI embedded): site 482,831 B; factory 252,631 B; escrow 257,875 B; mock-ft
  172,779 B.
- Measured claim: **3,714 parity vector executions** (3,024 + 690), 10 parity + 26 terms tests,
  **0 failed**; wasm sizes above. Gas is not claimed as a headline; the sandbox can print per-receipt
  `gas_burnt` if a later pass needs it.

## Layout

```
near-stele/
|-- contracts/{site,factory,escrow,mock-ft}
|-- vectors/{price-vectors.jsonl,v2-vectors.jsonl,manifest.json}
|-- packages/{sdk,react}
|-- apps/demo
|-- scripts/{regen-vectors-from-origin.sh,sandbox-dry-run.mjs}
`-- tests/sandbox
```

## Toolchain pins

Rust >= 1.95 (edition 2024) · near-sdk 5.30.1 · near-sdk-contract-tools 4.0.0 · primitive-types
0.14.0 · near-api-js 7.5.3 · near-sandbox 0.3.5 · vitest 4.1.10 · Node >= 22.22.2 · pnpm >= 10.4.1 ·
cargo-near 0.23.0 · near-cli-rs 0.31.1.

## Environment

`.env.example` carries `NEAR_FACTORY_ACCOUNT`, `NEAR_SITE_ACCOUNT`, `NEAR_VAULT_ACCOUNT`,
`NEAR_FT_ACCOUNT`, `NEAR_RPC_URL` (testnet example values, no keys).

## Limits

- Escrowed-site creation and timestamp-based expiry tests are descoped to W4 (see above).
- The sandbox is Unix-only and runs zero-funds; no mainnet deployment is part of this repository.
- The site's `min_slot_floor` is supplied by the factory from the settlement asset's real decimals
  (native 24 ⇒ `1e20`; 6-decimal mock FT ⇒ `100`); it is never derived from `ft_metadata` inside
  `new`, because that call is asynchronous.
- Royalty info is display-only; `buy` never deducts it.

## Origin and license

Port of an MIT-licensed EVM product; provenance, baseline commit and archive hash are in
`ORIGIN.md`, and the upstream notice is reproduced in `ATTRIBUTION.md`. MIT license kept. There is no
git remote; publishing is a human action.
