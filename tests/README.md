# Sandbox tests

Local, zero-funds integration loop over one ephemeral nearcore process started
by [`near-sandbox`](https://www.npmjs.com/package/near-sandbox) 0.3.5. The
harness creates `test.near`, deploys the four contracts, drives the market,
tenancy and escrow flows, and never calls mainnet or testnet RPC.

## Prerequisites

- Node.js `>=26.13.1` and pnpm `>=12.18.0` (fixed by `near-api-js` 7.5.3).
- Rust `>=1.100.0` and `cargo-near` 0.23.0.
- Linux, macOS, or **WSL2 on Windows**. `near-sandbox` ships Linux/macOS
  binaries only; on win32 clone under `/home/...` (not `/mnt/c/...`) and run
  the commands below inside WSL2. `solc`, Foundry and anvil are not needed.
- First run downloads the pinned nearcore binary (`2.14.0`) once from the
  `near-sandbox` cache. Everything after that is loopback RPC.

## Install and build

```bash
pnpm install

(cd contracts/mock-ft && cargo near build)
(cd contracts/site && cargo near build)
(cd contracts/factory && cargo near build)
(cd contracts/escrow && cargo near build)
```

`cargo near build` writes to `contracts/<crate>/target/near/`; a plain
`cargo build --target wasm32-unknown-unknown --release` writes to
`contracts/<crate>/target/wasm32-unknown-unknown/release/`. The harness reads
both, and accepts an explicit directory through `STELE_WASM_DIR`.

## Run

```bash
node scripts/sandbox-dry-run.mjs
```

prints one line per step (with every receipt status) and ends with:

```
DRY-RUN OK
```

Exit status is nonzero on the first failed assertion. To run the same loop as
a Vitest suite:

```bash
pnpm test:sandbox
```

## Runtime environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `STELE_WASM_DIR` | unset | One directory containing all four artifacts named `stele_site.wasm`, `stele_factory.wasm`, `stele_escrow.wasm`, `stele_mock_ft.wasm` (the unhyphenated `site.wasm` etc. also resolve). When unset the harness uses each contract's build directory. |
| `STELE_SITE_WASM` | unset | Explicit path for the site wasm; overrides `STELE_WASM_DIR` for that contract. |
| `STELE_FACTORY_WASM` | unset | Explicit path for the factory wasm. |
| `STELE_ESCROW_WASM` | unset | Explicit path for the escrow wasm. |
| `STELE_MOCK_FT_WASM` | unset | Explicit path for the mock FT wasm. |
| `NEAR_SANDBOX_VERSION` | `2.14.0` | nearcore build requested from `near-sandbox`. |
| `NEAR_SANDBOX_BIN_PATH` | unset | Use an already-downloaded `near-sandbox` binary instead of the package cache. |
| `DIR_TO_DOWNLOAD_BINARY` | `<near-sandbox>/bin` | Override the download root used by `near-sandbox`. |
| `NEAR_ENABLE_SANDBOX_LOG` | unset | `1` streams the nearcore node log to stderr. |

The sandbox is always bound to `127.0.0.1` on an ephemeral port; the harness
reads its URL from the `near-sandbox` handle and has no other network path.
No keys or credentials are required or stored: the root key is near-sandbox's
own public genesis key, and every other account is generated in memory.
