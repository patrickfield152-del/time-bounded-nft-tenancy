# SUBMISSION-PREP — stele (LOCAL DRAFT, NOT SUBMITTED)

Tracks: `nfts`, `realworld`. Deadline 2026-11-14T23:59:00Z.
No push, no remote, no tags, no X post, no submit. Local commit only.

## Judge quickstart (do X, see Y), MEASURED pre-Celesto-only

```
pnpm -C packages/sdk typecheck     -> EXIT 0
pnpm -C packages/react typecheck   -> EXIT 0
pnpm exec vitest run packages/sdk/src -> 1 file / 15 tests passed
cargo test -p stele-site --test parity -> 10 passed; 0 failed
cargo test -p stele-site --test terms  -> 26 passed; 0 failed
cargo test -p stele-site --test views  -> 2 passed; 0 failed
cargo check --workspace --target wasm32-unknown-unknown -> EXIT 0
                                  (rustc 1.100.0, near-sdk 5.30.2)
pnpm -C apps/demo build            -> EXIT 0 (Next 16.4.6)
pnpm -C packages/sdk build / react build -> EXIT 0 (tsup 9)
pnpm dedupe --check                -> EXIT 0
git diff HEAD --check              -> EXIT 0; git status --short empty
```

`compute_split` / payout-site rounding UNTOUCHED (OB-B 1,151 held).

## Recordable demo outline (human records, human posts)

1. Mint a slot NFT, write content behind the slot, read it back.
2. Rent the slot: issue a time-bounded function-call access key, sign an `edit`.
3. Let the term expire. Show the same signed call rejected.

One moment, one visible result. No screenshots-only demo (Q-012). Tags @NEARProtocol, #NEARHackathon. Main writes no post.

## Payload fields — ALL EMPTY, human supplies every value

| Field | Rule | Value |
|---|---|---|
| requestId | UUID, generated ONCE, reused on 400/503 | EMPTY |
| projectName | 2-80 chars | EMPTY |
| leadName | 2-80 chars | EMPTY |
| email | valid, <=160, max 2 submissions per address (Q-015) | EMPTY |
| size | `1` / `2 - 4` / `4 - 6` / `6 - 8` / `8+` | EMPTY |
| tracks | >=1 track id, each once; `nfts`, `realworld` fit | EMPTY |
| nearAddress | 2-64 lowercase mainnet account the team creates | EMPTY |
| projectUrl | http(s), <=300, public repo, unique | EMPTY |
| demoUrl | http(s), <=300, published X post | EMPTY |
| idea | 20-2000 chars | EMPTY |
| consent | `true` only after verbatim notice + human yes | EMPTY |

Never add `website`. Print full JSON, wait for explicit yes. 201 saved; 400 fix `fields` + same requestId; 403 stop; 409 stop; 413 shorten; 503 retry same requestId.

## Rules checklist

- Q-003: `origin-baseline` tag exists (verified). No `pre-hackathon` / `hackathon-submission` pair exists, and no README diff link. **Not satisfied**; local tags deliberately not created.
- Q-012 video required when judges cannot open the demo.
- Q-014 `LICENSE` present. Q-017 README Quickstart at line 72.
- Q-015 second project on same email must be named in README. Unresolved.
- Origin/license disclosure: README "Origin, license, hackathon" at line 142.

## Blockers (do not block submission)

Celesto `Trial credit is too low to start this sandbox` → clippy/fmt/cargo-near-check BLOCKED-BY-CELESTO. Firecrawl credits BLOCKED. Exa MCP 402 → direct REST. BoxLite §6 MEASURED `qvd8ZYJlMsng` (2 401 quoting failures recorded verbatim before the successful run).

## Attestation

Nothing published, pushed, tagged, or submitted. No payload value invented.
