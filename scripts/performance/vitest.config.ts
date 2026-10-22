import { defineConfig } from "vitest/config";
export default defineConfig({ test: { environment: "node", include: ["scripts/performance/frontend.test.ts"] } });
