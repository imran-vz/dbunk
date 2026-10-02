// Plan 024 measurement builds only. Wraps the project Vite config (or, with
// PLAN024_BRIDGE=1, the Plan 022 driver config that adds the evaluation
// bridge) and lets the prerender preview server take any free port. The
// project config pins port 3000 with `strictPort`, the preview inherits the
// strictness and is started on 3000 whatever port it is asked for, so a
// build fails whenever anything else on the machine is listening there.
// Never used by `pnpm dev` or a release build.
//   pnpm vite build --config tools/measure/tauri/vite.config.ts
import { defineConfig, type UserConfig } from "vite";

import bridge from "../../../infrastructure/test-db/schema-compare/webview-driver/vite.config";
import project from "../../../vite.config";

const PORT = 3024;

export default defineConfig(async (env) => {
  const base = process.env.PLAN024_BRIDGE === "1" ? bridge : project;
  const resolved: UserConfig =
    typeof base === "function" ? await base(env) : await base;
  return {
    ...resolved,
    server: { ...resolved.server, port: PORT },
    preview: { ...resolved.preview, port: PORT, strictPort: false },
  };
});
