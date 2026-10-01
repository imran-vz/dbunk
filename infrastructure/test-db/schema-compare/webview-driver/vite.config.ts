// Plan 022 native walkthrough driver. Wraps the project Vite config and adds a
// dev-only evaluation bridge over Vite's HMR channel so the real Tauri WebView
// can be driven from a shell when macOS denies assistive access. Never used by
// `pnpm dev` or any build; run it explicitly:
//   pnpm vite dev --port 3000 --config infrastructure/test-db/schema-compare/webview-driver/vite.config.ts
// then `curl -s localhost:3000/__dbunk/eval --data-binary '<js expression>'`.
// `vite build` with this config produces a production bundle that carries the
// same bridge; serve.py serves it. Memory and timing are measured there,
// because a development React build retains a User Timing entry per render.
import type { IncomingMessage } from "node:http";

import { defineConfig, type Plugin, type UserConfig } from "vite";

import base from "../../../../vite.config";

const VIRTUAL = "virtual:dbunk-driver";
const RESOLVED = "\0" + VIRTUAL;

const clientModule = `
// The app's own module instances, for the walkthrough helpers. Lazy, so the
// driver never changes the order in which application modules evaluate.
if (typeof window !== "undefined") {
  window.__dbunkModules = async () => ({
    store: (await import("/src/lib/store/index.ts")).useAppStore,
    observer: (await import("/src/lib/pg-schema-compare/observer.ts")).pgSchemaCompareObserver,
    form: (await import("/src/components/pg-schema-compare/use-compare-form.ts")).schemaCompareForm,
    client: (await import("/src/lib/pg-schema-compare/client.ts")).schemaCompareClient,
    forms: await import("/src/components/connection-form/form-utils.ts"),
    density: await import("/src/lib/density.ts"),
  });
}
const run = async (expr) => {
  try {
    let value = (0, eval)(expr);
    if (typeof value === "function") value = value();
    value = await value;
    return { ok: true, value: value === undefined ? null : value };
  } catch (error) {
    return { ok: false, error: error && error.message ? \`\${error.name}: \${error.message}\n\${error.stack ?? ""}\` : String(error) };
  }
};
if (typeof window !== "undefined" && import.meta.hot) {
  // Dev server: Vite's HMR channel carries commands and results.
  import.meta.hot.on("dbunk:eval", async ({ id, expr }) => {
    import.meta.hot.send("dbunk:result", { id, ...(await run(expr)) });
  });
  import.meta.hot.send("dbunk:hello", { href: location.href, ua: navigator.userAgent });
} else if (typeof window !== "undefined") {
  // Built bundle: long-poll the static bridge server (serve.py).
  const post = (path, body) => fetch(path, { method: "POST", body: JSON.stringify(body) });
  post("/__dbunk/hello", { href: location.href, ua: navigator.userAgent, built: true, at: new Date().toISOString() });
  (async () => {
    for (;;) {
      try {
        const response = await fetch("/__dbunk/next");
        if (response.status !== 200) continue;
        const { id, expr } = await response.json();
        await post("/__dbunk/result", { id, ...(await run(expr)) });
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
    }
  })();
}
`;

function readBody(request: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    request.on("error", reject);
  });
}

function driver(): Plugin {
  const pending = new Map<
    string,
    { resolve: (value: unknown) => void; timer: NodeJS.Timeout }
  >();
  let hello: unknown = null;
  let counter = 0;
  return {
    name: "dbunk-webview-driver",
    resolveId(id) {
      return id === VIRTUAL ? RESOLVED : undefined;
    },
    load(id) {
      return id === RESOLVED ? clientModule : undefined;
    },
    transform(code, id) {
      // The router module is the first app module the client evaluates.
      // Client environment only: the SSR module runner has no window.
      return id.endsWith("/src/router.tsx") &&
        this.environment.name === "client"
        ? `import "${VIRTUAL}";\n${code}`
        : undefined;
    },
    configureServer(server) {
      server.ws.on("dbunk:hello", (data) => {
        hello = { ...(data as object), at: new Date().toISOString() };
      });
      server.ws.on("dbunk:result", (data: { id: string }) => {
        const entry = pending.get(data.id);
        if (!entry) return;
        clearTimeout(entry.timer);
        pending.delete(data.id);
        entry.resolve(data);
      });
      server.middlewares.use("/__dbunk/status", (_request, response) => {
        response.setHeader("content-type", "application/json");
        response.end(JSON.stringify({ hello, pending: pending.size }));
      });
      server.middlewares.use("/__dbunk/eval", async (request, response) => {
        const expr = await readBody(request);
        const id = String(++counter);
        const timeoutMs = Number(request.headers["x-timeout-ms"] ?? 60000);
        const result = await new Promise<unknown>((resolve) => {
          const timer = setTimeout(() => {
            pending.delete(id);
            resolve({ id, ok: false, error: `timeout after ${timeoutMs}ms` });
          }, timeoutMs);
          pending.set(id, { resolve, timer });
          server.ws.send({
            type: "custom",
            event: "dbunk:eval",
            data: { id, expr },
          });
        });
        response.setHeader("content-type", "application/json");
        response.end(JSON.stringify(result));
      });
    },
  };
}

export default defineConfig(async (env) => {
  const resolved: UserConfig =
    typeof base === "function" ? await base(env) : await base;
  return { ...resolved, plugins: [...(resolved.plugins ?? []), driver()] };
});
