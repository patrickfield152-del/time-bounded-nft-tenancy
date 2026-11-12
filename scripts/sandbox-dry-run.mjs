#!/usr/bin/env node
//! Local-only stele dry-run: boot one `near-sandbox` node, deploy the four
//! contracts, drive the market/tenancy/FT/escrow loop, and exit nonzero on the
//! first failed assertion. Every account and token is created inside the
//! sandbox; no mainnet or testnet RPC is reachable from this process.

import { assertSandboxBinary, SandboxHarness } from "../tests/sandbox/harness.mjs";
import { runFlows } from "../tests/sandbox/flows.mjs";

async function main() {
  const harness = await SandboxHarness.start();
  try {
    const binary = await assertSandboxBinary();
    console.log(`[sandbox] node ${harness.rpcUrl} home=${harness.homeDir}`);
    console.log(`[sandbox] binary ${binary}`);
    await runFlows(harness);
    console.log("DRY-RUN OK");
  } finally {
    await harness.stop();
  }
}

main().catch((error) => {
  console.error("[sandbox] FAILED");
  console.error(error);
  process.exitCode = 1;
});
