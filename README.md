# stele

**Tracks:** `nfts`, `realworld` · **Licence:** MIT · **NEP:** NEP-171 / NEP-141 · **Baseline:** tag `pre-hackathon` marks the imported tree at `30779ef` (upstream `vibecodermaxi/websitekit`), diff `git diff pre-hackathon..hackathon-submission` · **Q-015 sibling:** **skjold** (agent identity and credentials, tracks `security`, `agents`).

Slot NFTs that do something: each NEP-171 token is a site slot with a price curve and a pull-payment ledger. Buys are guarded by deadline, terms, cooldown and slippage; terms only tighten; sellers withdraw from escrow credit. Tenancy expiry is descoped.

```mermaid
sequenceDiagram
    accTitle: stele runtime flow
    accDescr: Seven phases from code deploy to slot mint: escrow bind, vault funding, guarded buy, split payout.
    autonumber
    participant D as Demo
    participant S as SDK
    participant F as Factory
    participant T as Site
    participant E as Escrow
    participant M as MockFT
    Note over D,F: PHASE 1 — Deploy code and params once
    D->>+F: store_site_code(code)
    D->>F: store_escrow_code(code)
    D->>F: set_escrow_params(params) and allow_token(token, decimals)
    F-->>-D: SiteCodeNotStored released
    Note over D,T: PHASE 2 — Create site and vault
    D->>S: buildCreateSite(options)
    S->>+F: create_escrowed_site(vault, site, config)
    F->>F: must_be_child and floor_for checks
    F->>E: create_account then new(factory)
    F->>T: create_account then new(owner, treasury, config)
    F-->>-D: SiteCreated event
    Note over F,E: PHASE 3 — Bind escrow to treasury
    F->>T: storage_deposit(vault_id)
    F->>+E: bind(site, escrow_params)
    E->>T: terms()
    E-->>-F: bind_callback(terms)
    E->>T: own_get_owner()
    E->>E: bind_owner_callback caches publisher, book_inner
    F-->>F: escrowed_bind_callback records vault_of, refunds change
    Note over M,E: PHASE 4 — Fund vault via settlement token
    S->>M: ft_transfer_call(amount, msg)
    M->>E: ft_on_transfer(amount, msg)
    E->>E: push_deposit or reserve, other msg refunds
    D->>E: release(deposit_id) after is_mature
    E->>E: credit(beneficiary) then advance_head
    Note over D,T: PHASE 5 — Buy with terms guard
    S->>+T: buy(token_id, max_price, expected_terms, deadline_ns)
    T->>T: plan_buy DeadlineExpired, TermsChanged, CooldownActive, SlippageExceeded
    Note over T,M: PHASE 6 — Settle split payout
    alt native deposit
    T->>+T: storage_top_up then storage_deposit
    T-->>-T: settle_buy_callback
    else settlement token
    M->>T: ft_on_transfer dispatch, price never exceeds max_price
    end
    T->>T: execute_buy settle_rent then credit payout, protocol_cut and site_cut
    Note over D,T: PHASE 7 — Mint slot and emit events
    T->>T: mint_with_metadata or external_transfer, listing cleared
    T-->>-D: SlotBought and TermsLocked events
```

```bash
cargo check --workspace --target wasm32-unknown-unknown
cargo test -p stele-site --tests
for c in site factory escrow mock-ft; do (cd contracts/$c && cargo near build non-reproducible-wasm); done
pnpm install
pnpm -C packages/sdk typecheck && pnpm -C packages/react typecheck
pnpm -C packages/sdk build && pnpm -C packages/react build
pnpm exec vitest run packages/sdk/src
STELE_SKIP_ESCROW=1 STELE_SKIP_EXPIRY=1 node scripts/sandbox-dry-run.mjs
```

| Do this | See this | Proves |
|---|---|---|
| run the quickstart block above | clean builds | quickstart |
| `cargo test -p stele-site --tests` | 38 tests pass | tests |
| `pnpm exec vitest run packages/sdk/src` | 15 tests pass | SDK tested |
| `vectors/manifest.json` | 3,714 vector executions | price curve parity |
| buy guards (`plan_buy`) | DeadlineExpired/TermsChanged/CooldownActive/SlippageExceeded | guards are tested |
| `node scripts/sandbox-dry-run.mjs` | `DRY-RUN OK` | smoke/measure |
| `git diff pre-hackathon..hackathon-submission`; `git rev-parse --short hackathon-submission` | tag `pre-hackathon` at `30779ef` | baseline |
| descoped dry-run | escrowed-site creation and timestamp-based expiry are descoped | Limits: `create_escrowed_site` fails at i=3 with `Exceeded the prepaid gas` (12.808 TGas); royalties are display-only |

Links: [ATTRIBUTION](ATTRIBUTION.md) · [LICENSE](LICENSE).
