import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    testTimeout: 30_000,
    // One shared backend; every test makes its own accounts and organizations, so no cleanup is needed.
    fileParallelism: false,
  },
});
