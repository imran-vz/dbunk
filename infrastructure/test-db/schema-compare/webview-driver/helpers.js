// Plan 022 walkthrough helpers, evaluated in the real WebView by walkthrough.py.
// They click and read the rendered workspace; state is only inspected to wait
// for a settled view and to count retained payloads.
(() => {
  const h = (window.__h = {});
  const TERMINAL = ["completed", "failed", "cancelled"];

  h.sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  h.until = async (probe, ms = 15000, step = 25) => {
    const end = performance.now() + ms;
    for (;;) {
      const value = await probe();
      if (value) return value;
      if (performance.now() > end)
        throw new Error(`timeout waiting for ${probe}`);
      await h.sleep(step);
    }
  };
  h.root = () =>
    document.querySelector('[data-testid="pg-schema-compare-workspace"]');
  h.text = () => (h.root() ? h.root().innerText : "(workspace not mounted)");
  h.byText = (selector, text, scope = document) =>
    [...scope.querySelectorAll(selector)].find(
      (element) => element.textContent.trim() === text,
    );
  h.byLabel = (label, scope = document) =>
    scope.querySelector(`[aria-label="${label}"]`);
  h.click = (element, what = "element") => {
    if (!element) throw new Error(`nothing to click: ${what}`);
    element.click();
  };
  h.set = (element, value) => {
    const prototype =
      element instanceof HTMLSelectElement
        ? HTMLSelectElement.prototype
        : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value").set.call(
      element,
      value,
    );
    element.dispatchEvent(
      new Event(element instanceof HTMLSelectElement ? "change" : "input", {
        bubbles: true,
      }),
    );
  };
  // Module instances come from the driver bundle (vite.config.ts), so the same
  // helpers work against the dev server and a production build.
  h.mods = () => window.__dbunkModules();
  h.window = (command, args = {}) =>
    window.__TAURI_INTERNALS__.invoke(`plugin:window|${command}`, {
      label: "main",
      ...args,
    });

  /** Opens the Schema compare rail destination for a connected connection. */
  h.open = async (connectionId) => {
    const { store } = await h.mods();
    await h.until(() =>
      store.getState().connections.some((c) => c.id === connectionId),
    );
    store.getState().setActiveConnectionId(connectionId);
    const status = () =>
      store.getState().connections.find((c) => c.id === connectionId).status;
    if (status() !== "Connected")
      await store.getState().connectConnection(connectionId);
    await h.until(() => h.byLabel("Schema compare"));
    if (!h.root()) h.click(h.byLabel("Schema compare"));
    await h.until(() => h.root());
    return status();
  };
  h.rail = async (label) => {
    h.click(h.byLabel(label), `rail ${label}`);
    await h.sleep(0);
  };

  h.endpoints = async (source, sourceSchema, target, targetSchema) => {
    h.set(h.byLabel("Source connection"), source);
    h.set(h.byLabel("Target connection"), target);
    // Changing a connection clears its schema; let that render first.
    await h.sleep(60);
    h.set(h.byLabel("Source schema"), sourceSchema);
    h.set(h.byLabel("Target schema"), targetSchema);
    await h.sleep(60);
  };
  h.compareButton = () =>
    [...h.root().querySelectorAll("button")].find((b) =>
      /^(Compare|Starting…)$/.test(b.textContent.trim()),
    );
  /** Fills the form and presses Compare; optionally waits for a settled result. */
  h.compare = async (
    [source, sourceSchema, target, targetSchema],
    { wait = true } = {},
  ) => {
    const { observer, form } = await h.mods();
    const before = new Set(
      observer.store.getState().jobs.map((job) => job.jobId),
    );
    await h.endpoints(source, sourceSchema, target, targetSchema);
    const started = performance.now();
    h.click(h.compareButton(), "Compare");
    const phases = [];
    let job;
    const seen = () => {
      job = observer.store
        .getState()
        .jobs.find((candidate) => !before.has(candidate.jobId));
      if (job && phases.at(-1) !== job.phase) phases.push(job.phase);
      return job;
    };
    await h.until(() => seen() || form.getState().error, 15000, 10);
    if (!job) return { formError: form.getState().error, text: h.text() };
    if (!wait) return { jobId: job.jobId, phases };
    await h.until(() => seen() && TERMINAL.includes(job.phase), 75000, 20);
    const ms = Math.round(performance.now() - started);
    if (job.phase === "completed") {
      await h.until(
        () =>
          h.byLabel("Compared objects") ||
          /No ordinary tables|unavailable|Retry/.test(h.text()),
        15000,
      );
    }
    await h.sleep(200);
    return { ms, phases, job, text: h.text() };
  };
  h.waitTerminal = async (jobId, ms = 75000) => {
    const { observer } = await h.mods();
    const phases = [];
    let job;
    await h.until(
      () => {
        job = observer.store
          .getState()
          .jobs.find((candidate) => candidate.jobId === jobId);
        if (job && phases.at(-1) !== job.phase) phases.push(job.phase);
        return !job || TERMINAL.includes(job.phase);
      },
      ms,
      20,
    );
    await h.sleep(200);
    return { phases, job: job ?? null, text: h.text() };
  };

  h.objects = () =>
    Object.fromEntries(
      [
        ...(h.byLabel("Compared objects")?.querySelectorAll("li button") ?? []),
      ].map((button) => [
        button.firstElementChild.textContent,
        button.lastElementChild.textContent,
      ]),
    );
  // Waits read only small nodes. Reading innerText of a pane that holds a
  // 64 KiB value forces layout and copies the text on every poll, which would
  // distort the timing and memory this harness is here to measure.
  const PENDING =
    /^(Reading (fields|objects|result metadata)…|Loading…|Not loaded|Fields not loaded\.)$|eligibility not loaded/;
  h.pending = () =>
    Boolean(h.root().querySelector(".animate-loading-bar")) ||
    [...h.root().querySelectorAll("p, span.text-text-muted")].some(
      (node) =>
        node.childElementCount <= 1 &&
        node.textContent.length < 80 &&
        PENDING.test(node.textContent.trim()),
    );
  h.settled = () => h.until(() => !h.pending(), 15000);
  h.selectedHeader = () =>
    h.byLabel("Selected object fields")?.querySelector("span.font-semibold")
      ?.textContent ?? "";
  h.object = async (name) => {
    const button = [
      ...h.byLabel("Compared objects").querySelectorAll("li button"),
    ].find((candidate) => candidate.firstElementChild.textContent === name);
    h.click(button, `object ${name}`);
    await h.until(() => h.selectedHeader().endsWith(`.${name}`));
    await h.settled();
    return h.byLabel("Selected object fields").innerText;
  };
  h.fieldRows = () =>
    [
      ...(h.byLabel("Selected object fields")?.querySelectorAll("tbody tr") ??
        []),
    ].map((row) =>
      [...row.cells].map((cell) => cell.innerText.replace(/\s+/g, " ").trim()),
    );
  /** Selects a field row; `read: false` skips copying the inspector text. */
  h.field = async (match, read = true) => {
    const rows = [
      ...h.byLabel("Selected object fields").querySelectorAll("tbody tr"),
    ];
    const label = (row) => row.cells[0].textContent.replace(/\s+/g, " ").trim();
    const key = match.replace(/\s+/g, "");
    const row = rows.find((candidate) =>
      label(candidate).replace(/\s+/g, "").startsWith(key),
    );
    if (!row) throw new Error(`no field ${match}`);
    h.click(row.querySelector("button"), `field ${match}`);
    await h.until(
      () =>
        row.querySelector('button[aria-pressed="true"]') &&
        h.byLabel("Value inspector"),
    );
    await h.sleep(40);
    await h.settled();
    if (!read) return null;
    return {
      row: [...row.cells].map((cell) =>
        cell.innerText.replace(/\s+/g, " ").trim(),
      ),
      inspector: h.byLabel("Value inspector").innerText,
    };
  };
  h.position = (label) =>
    h.byLabel(`Next ${label} page`, h.root())?.previousElementSibling
      ?.textContent ?? null;
  h.page = async (label, direction) => {
    const button = h.byLabel(`${direction} ${label} page`, h.root());
    if (!button || button.disabled) return false;
    const before = h.position(label);
    button.click();
    await h.until(() => h.position(label) !== before);
    await h.settled();
    return true;
  };
  h.pane = (side) => h.byLabel(`${side} value`);
  h.paneHead = (side) =>
    [...h.pane(side).firstElementChild.children]
      .map((node) => node.textContent.trim())
      .join(" ");
  h.chunk = async (side, direction) => {
    const button = h.byText("button", `${direction} chunk`, h.pane(side));
    if (!button || button.disabled) return false;
    const before = h.paneHead(side);
    button.click();
    await h.until(
      () => h.pane(side).querySelector("pre") && h.paneHead(side) !== before,
    );
    await h.settled();
    return true;
  };
  h.jobRows = () =>
    [...h.root().querySelectorAll("details li")].filter((row) =>
      row.querySelector("button[aria-pressed]"),
    );
  /** Selects the session job with these schemas through its row. */
  h.selectJob = async (sourceSchema, targetSchema) => {
    const { observer } = await h.mods();
    const index = observer.store
      .getState()
      .jobs.findIndex(
        (job) =>
          job.source.schema === sourceSchema &&
          job.target.schema === targetSchema,
      );
    h.click(
      h.jobRows()[index]?.querySelector("button[aria-pressed]"),
      `job ${sourceSchema} -> ${targetSchema}`,
    );
    await h.until(() =>
      h.jobRows()[index]?.querySelector('button[aria-pressed="true"]'),
    );
    await h.sleep(80);
    await h.settled();
    return observer.store.getState().jobs[index];
  };
  /** Selects a completed job and waits for its first object page. */
  h.openResult = async (sourceSchema, targetSchema) => {
    await h.selectJob(sourceSchema, targetSchema);
    await h.until(() => h.byLabel("Compared objects"), 15000);
    await h.settled();
  };
  /** Cancels or dismisses every session job through its own row button. */
  h.clearJobs = async () => {
    const { observer } = await h.mods();
    for (
      let round = 0;
      round < 12 && observer.store.getState().jobs.length;
      round++
    ) {
      for (const row of h.jobRows()) {
        const button = [...row.querySelectorAll("button")].find((b) =>
          /^(Cancel|Dismiss)$/.test(b.textContent.trim()),
        );
        if (button && !button.disabled) button.click();
      }
      await h.sleep(400);
      await observer.refresh();
    }
    return observer.store.getState().jobs.length;
  };

  // Tauri locks `invoke`; on macOS its transport is fetch("ipc://localhost/<command>").
  if (!window.__trace) {
    const original = window.fetch;
    const trace = (window.__trace = {
      calls: [],
      inflightReads: 0,
      maxInflightReads: 0,
      unacked: new Set(),
      maxUnacked: 0,
      reset() {
        this.calls = [];
        this.maxInflightReads = this.inflightReads;
        this.maxUnacked = this.unacked.size;
      },
    });
    window.fetch = async function (input, init) {
      const url = typeof input === "string" ? input : input.url;
      const command = url.startsWith("ipc://localhost/")
        ? decodeURIComponent(url.slice(16))
        : "";
      if (!command.includes("pg_schema_compare"))
        return original.call(this, input, init);
      let args = {};
      try {
        const body = init?.body;
        args = JSON.parse(
          typeof body === "string" ? body : new TextDecoder().decode(body),
        );
      } catch {}
      const started = performance.now();
      const read = command === "read_pg_schema_compare";
      if (read) {
        trace.inflightReads++;
        trace.maxInflightReads = Math.max(
          trace.maxInflightReads,
          trace.inflightReads,
        );
      }
      const record = {
        command: command.replace("_pg_schema_compare", ""),
        kind: read ? args.read?.kind : undefined,
      };
      try {
        const response = await original.call(this, input, init);
        record.ok =
          response.ok && response.headers.get("Tauri-Response") !== "error";
        const length = response.headers.get("content-length");
        if (length) record.bytes = Number(length);
        if (read && record.ok) {
          trace.unacked.add(args.responseId);
          trace.maxUnacked = Math.max(trace.maxUnacked, trace.unacked.size);
        }
        if (command === "acknowledge_pg_schema_compare" && record.ok)
          trace.unacked.delete(args.responseId);
        return response;
      } catch (error) {
        record.ok = false;
        throw error;
      } finally {
        if (read) trace.inflightReads--;
        record.ms = Math.round((performance.now() - started) * 10) / 10;
        trace.calls.push(record);
        if (trace.calls.length > 20000) trace.calls.splice(0, 10000);
      }
    };
  }
  h.traceSummary = () => {
    const by = {};
    for (const call of window.__trace.calls) {
      const key = call.command + (call.kind ? `:${call.kind}` : "");
      const entry = (by[key] ??= { n: 0, failed: 0, maxMs: 0, maxBytes: 0 });
      entry.n++;
      if (!call.ok) entry.failed++;
      entry.maxMs = Math.max(entry.maxMs, call.ms);
      entry.maxBytes = Math.max(entry.maxBytes, call.bytes ?? 0);
    }
    return {
      by,
      maxInflightReads: window.__trace.maxInflightReads,
      unackedNow: window.__trace.unacked.size,
      maxUnacked: window.__trace.maxUnacked,
    };
  };

  /** Rendered payload census: counts only, never contents. */
  h.census = () => {
    const root = h.root();
    if (!root) return null;
    return {
      objectRows:
        h.byLabel("Compared objects")?.querySelectorAll("li").length ?? 0,
      fieldRows:
        h.byLabel("Selected object fields")?.querySelectorAll("tbody tr")
          .length ?? 0,
      valueBlocks: root.querySelectorAll('section[aria-label$=" value"] pre')
        .length,
      valueChars: [
        ...root.querySelectorAll('section[aria-label$=" value"] pre'),
      ].reduce((sum, pre) => sum + pre.textContent.length, 0),
      domNodes: root.querySelectorAll("*").length,
    };
  };

  // Frame pacing while the page is visible: gaps between animation frames.
  h.frames = {
    start() {
      const state = (h.frames.state = {
        last: performance.now(),
        n: 0,
        over50: 0,
        over100: 0,
        max: 0,
        on: true,
      });
      const tick = (now) => {
        if (!state.on) return;
        const gap = now - state.last;
        state.last = now;
        state.n++;
        if (gap > 50) state.over50++;
        if (gap > 100) state.over100++;
        state.max = Math.max(state.max, Math.round(gap));
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    },
    stop() {
      const { n, over50, over100, max } = h.frames.state;
      h.frames.state.on = false;
      return { frames: n, over50ms: over50, over100ms: over100, maxGapMs: max };
    },
  };
  return "helpers installed";
})();
