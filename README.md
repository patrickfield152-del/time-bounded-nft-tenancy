# stele

**NEP-171 slot NFTs + time-bounded function-call-key tenancy, NEP-141 settlement.**

A publisher divides a page into named slots. Each slot is a NEP-171 token. The holder can write the content behind the slot. The holder can also sell the slot on the site's own mean-reverting price curve. A holder can rent the slot for a fixed term. Tenancy is a time-bounded function-call access key. The tenant signs content writes with a session key scoped to the site contract and to `edit`. A function-call key carries no expiry of its own, so the contract asserts the rental window on every gated call. Settlement is NEP-141, and the native branch pays native NEAR. Payouts go through a pull ledger. Every site is its own account, with its own storage and its own frozen code.

```mermaid
%%{init: {"theme":"base","themeVariables":{"fontSize":"14px"}}}%%
sequenceDiagram
    autonumber
    participant D as Demo
    participant S as SDK
    participant F as Factory
    participant T as Site
    participant E as Escrow
    participant M as MockFT
    rect rgb(226,238,255)
    Note over D,F: PHASE 1 — Deploy: factory stores code and escrow params once
    D->>F: store_site_code(code)
    D->>F: store_escrow_code(code)
    D->>F: set_escrow_params(params) and allow_token(token, decimals)
    F-->>D: SiteCodeNotStored released
    end
    rect rgb(230,247,235)
    Note over D,T: PHASE 2 — Create site account: vault and site deployed as factory children
    D->>S: buildCreateSite(options)
    S->>F: create_escrowed_site(vault, site, config)
    F->>F: must_be_child and floor_for checks
    F->>E: create_account then new(factory)
    F->>T: create_account then new(owner, treasury, config)
    alt direct site
    S->>F: create_site(payload)
    end
    F-->>D: SiteCreated event
    end
    rect rgb(255,244,214)
    Note over F,E: PHASE 3 — Bind escrow: pinned treasury and params committed before booking
    F->>T: storage_deposit(vault_id)
    F->>E: bind(site, escrow_params)
    E->>T: terms()
    E-->>F: bind_callback(terms)
    E->>T: own_get_owner()
    E->>E: bind_owner_callback caches publisher, book_inner
    F-->>F: escrowed_bind_callback records vault_of, refunds change
    end
    rect rgb(243,232,255)
    Note over M,E: PHASE 4 — Fund vault: only the bound settlement token may book or reserve
    S->>M: ft_transfer_call(amount, msg)
    M->>E: ft_on_transfer(amount, msg)
    E->>E: push_deposit or reserve, other msg refunds
    D->>E: release(deposit_id) after is_mature
    E->>E: credit(beneficiary) then advance_head
    end
    rect rgb(255,232,232)
    Note over D,T: PHASE 5 — Buy with terms guard: hash, cooldown and deadline gate every buy
    D->>S: Promise.all readSettlementAsset, readSlots, readSiteTerms
    D->>S: buildBuy(options)
    S->>T: buy(token_id, max_price, expected_terms, deadline_ns)
    T->>T: plan_buy DeadlineExpired, TermsChanged, CooldownActive, SlippageExceeded
    end
    rect rgb(224,247,250)
    Note over T,M: PHASE 6 — Settle split payout: credits book before ownership moves
    alt native deposit
    T->>T: storage_top_up then storage_deposit
    T-->>T: settle_buy_callback
    else settlement token
    M->>T: ft_on_transfer dispatch, price never exceeds max_price
    end
    T->>T: execute_buy settle_rent then credit payout, protocol_cut and site_cut
    end
    rect rgb(255,248,225)
    Note over D,T: PHASE 7 — NFT slot and events: first claim mints, every sale emits
    T->>T: mint_with_metadata or external_transfer, listing cleared
    T-->>D: SlotBought and TermsLocked events
    end
```

## Status
W1 REPO: scaffold, toolchain pins, state/type layouts. W2 PRIMITIVE: parity vectors, pricing/rentals/terms, site NFT core, SDK and React. Both done.

W3 FLOWS: tenancy/edit/claim, factory, escrow, mock FT, sandbox harness. Done, descoped below. W4 CLOSE: README, numbers, sweep, clean-room. Done.

## Quickstart
```bash
# 1. contracts (rustc 1.100.0, pinned by rust-toolchain.toml)
cargo check --workspace --target wasm32-unknown-unknown
cargo test -p stele-site --tests            # 38 tests
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
Vectors: **3,024** take-price rows + **690** v2 ask/rent/fee rows, with a sha256 per file in `vectors/manifest.json`.

- Parity: 10 tests, 0 failed (`cargo test -p stele-site --test parity`).
- Terms/ratchet: 26 tests, 0 failed (`cargo test -p stele-site --test terms`).
- View/encumbrance: 2 tests, 0 failed (`cargo test -p stele-site --test views`).
- SDK behavior, `packages/sdk/src/stele.test.ts`: 15 tests, 0 failed.

Measured claim: **3,714 parity vector executions**, 38 Rust + 15 SDK tests, **0 failed**. Gas is not a headline claim. The sandbox can print a per-receipt `gas_burnt` if a later pass needs it.

Release wasm sizes come from `cargo near build non-reproducible-wasm`. The ABI is embedded, `opt-level = "z"`, and factory/escrow/mock-ft are `cdylib`-only:

| Artifact | Bytes |
|---|---|
| site | 428,124 |
| factory | 183,047 |
| escrow | 179,537 |
| mock-ft | 125,535 |

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
- Rust 1.100.0, edition 2024, pinned by `rust-toolchain.toml`.
- near-sdk 5.30.2, near-sdk-contract-tools 4.0.0, primitive-types 0.14.0, near-api-js 7.5.3, near-sandbox 0.3.5.
- @hot-labs/near-connect 0.12.0, near-connect-hooks 1.1.6, typescript 7.0.2, tsx 4.23.15, tsup 9.0.0.
- vitest 5.1.0, next 16.4.6, react/react-dom 19.3.1, @types/node 26.6.6.
- Node >= 26.13.1 (`.nvmrc`), pnpm 12.18.0, cargo-near 0.23.0, wasm-opt 0.116.1.

`pnpm-workspace.yaml` holds `autoDedupe`, `dedupePeers`, and `overrides`. The overrides keep exactly one `@hot-labs/near-connect` version in the lockfile. Lockfiles: `Cargo.lock` and `pnpm-lock.yaml`. `.env.example` carries `NEAR_FACTORY_ACCOUNT`, `NEAR_SITE_ACCOUNT`, `NEAR_VAULT_ACCOUNT`, `NEAR_FT_ACCOUNT`, and `NEAR_RPC_URL`: testnet example values, no keys.

## Limits
- Escrowed-site creation and timestamp-based expiry are descoped. `create_escrowed_site` fails in its vault/site XCC chain at receipt `i=3` with `Exceeded the prepaid gas` (12.808 TGas). The `+0.89 TGas` is fixed function-call and receipt-creation overhead. Attach at `allotment - 1 Tgas` instead of raising caps.
- `sandbox_fast_forward` moves height only, so block timestamps do not move. The post-expiry edit and `end_rental`/hash-restore assertions are unreachable in the harness. Patch `Rental.expiry`/`BlockInfo` instead of advancing height. The descoped dry-run (`STELE_SKIP_ESCROW=1 STELE_SKIP_EXPIRY=1`) ends `DRY-RUN OK`.
- The sandbox is Unix-only and runs zero-funds. No mainnet deployment is part of this repository.
- The factory supplies the site's `min_slot_floor` from the settlement asset's real decimals. Native 24 gives 1e20, and a 6-decimal mock FT gives 100. `new` never derives the floor from `ft_metadata`, because that call is asynchronous.
- Royalty info is display-only. `buy` never deducts it.

## Origin, license, hackathon
Port of an MIT-licensed EVM product. Provenance, baseline commit, and archive hash are in `ORIGIN.md`. `ATTRIBUTION.md` reproduces the upstream notice. The MIT license is kept. There is no git remote, and publishing is a human action.

Tracks: `realworld`, `business`. Development ends 2026-11-14T23:59:00Z (UTC).
