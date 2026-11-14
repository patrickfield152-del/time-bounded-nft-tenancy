import { defineConfig } from "vitest/config";

// The sandbox suite is a scripted .mjs loop, not a `*.test.*` module; the
// default include pattern would never collect it. One nearcore process per
// file, so file parallelism stays off.
export default defineConfig({
  test: {
    include: ["tests/sandbox/flows.mjs", "packages/sdk/src/**/*.test.ts"],
    pool: "threads",
    fileParallelism: false,
    testTimeout: 900_000,
    hookTimeout: 900_000,
  },
});
