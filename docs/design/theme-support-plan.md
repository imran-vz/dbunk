# Theme support — implementation plan

**Status**: Planned · 2026-05-14
**Origin**: P0-1 finding in [code-review 2026-05-14](../code-reviews/2026-05-14-tier-2-redis-and-ux.md) — the theme picker shipped in commit `0567311` is dead UI.

## Decisions (locked in)

| Question | Choice |
|---|---|
| Default mode for new installs | **System** — follow `prefers-color-scheme` live |
| Light palette | **Refined fresh** — not the `.export-light` colours |
| Persistence | **AppSettings / SQLite** now (combines former Phase 1 + Phase 3) |
| Implementation timing | **Plan only** — implement later |

---

## Current state (at time of writing)

- `styles.css` has:
  - `:root { ... }` — shadcn light defaults, **incomplete** (missing project-custom `--surface-*`, `--text-*`, `--accent-*`).
  - `.dark { ... }` — full dbunk dark palette from DESIGN.md §7. Used today as the only render path.
  - `.export-light { ... }` — complete light token set for image-export rendering. Stays as-is (different use case).
- `app-shell.tsx:137` unconditionally `documentElement.classList.add("dark")` on mount → forces dark regardless of menu choice.
- `app-preferences-menu.tsx` writes `data-theme="dark"|"light"` to `<html>` → nothing reads that selector → dead UI.
- Monaco editor theme (`vs-dark`/`vs`) is set once on mount and never re-evaluated.

## Architecture

### Theme model

```ts
type ThemeMode = "system" | "light" | "dark";
// Reserved for Phase 2:
// type ThemePreset = "default" | "dracula" | "github" | "gruvbox";
```

### DOM scheme (extensible without refactor)

- **Mode** drives the `.dark` class on `<html>`. Tailwind v4's `@custom-variant dark (&:is(.dark *))` already cascades to all `dark:` utilities.
- **Preset** (Phase 2) drives a `data-theme="dracula"` attribute. Each preset is a CSS class scope (`[data-theme="dracula"] { --surface-app: …; ... }`) that overrides the token set. Default preset = no attribute = uses `:root` / `.dark` tokens. Axes are orthogonal: `.dark[data-theme="dracula"]` works.

### Source of truth

- **AppSettings / SQLite** is canonical. `AppSettingsSnapshot.theme` carries the mode.
- **`localStorage["dbunk.theme"]`** is the boot cache — written every time the menu changes mode, read by an inline script before React mounts to avoid FOUC.

---

## Refined light palette

Drawn fresh to match DESIGN.md aesthetics rather than the `.export-light` palette (which targets PDF/PNG rendering and uses a different green). The cool, slightly-blue tinted surfaces echo the dark palette's identity inverted, with semantic tints rebalanced for legibility on light backgrounds.

```css
:root {
  /* Surfaces — cool off-whites, no pure white (eye-strain) */
  --surface-app:            #f7f9fb;  /* page background */
  --surface-window:         #ffffff;  /* main content panes */
  --surface-sidebar:        #f0f3f6;  /* sidebar tint */
  --surface-panel:          #ffffff;
  --surface-panel-elevated: #f6f8fa;  /* dropdowns, dialogs */
  --surface-input:          #ffffff;
  --surface-row:            #ffffff;
  --surface-row-hover:      #eef4ff;  /* subtle blue tint on hover */
  --accent-overlay:         #e6f4d8;  /* pale green wash */

  --border-subtle:          #e1e4e8;
  --border-strong:          #cbd1d8;

  --text-primary:           #1f2933;  /* near-black, cool */
  --text-secondary:         #4a5560;
  --text-muted:             #6b7681;
  --text-disabled:          #9aa3ad;

  /* Accent — darker green for contrast on light surfaces */
  --accent:           #3b9740;
  --accent-hover:     #2f7d32;
  --accent-subdued:   #e6f4d8;

  --semantic-success:       #3b9740;
  --semantic-warning:       #c97a17;
  --semantic-danger:        #d23535;
  --semantic-info:          #0969da;

  /* shadcn-compatible tokens — replaces the current incomplete :root block */
  --background:             var(--surface-app);
  --foreground:             var(--text-primary);
  --card:                   var(--surface-panel);
  --card-foreground:        var(--text-primary);
  --popover:                var(--surface-panel-elevated);
  --popover-foreground:     var(--text-primary);
  --primary:                var(--accent);
  --primary-foreground:     #ffffff;
  --secondary:              var(--surface-panel-elevated);
  --secondary-foreground:   var(--text-primary);
  --muted:                  var(--surface-panel-elevated);
  --muted-foreground:       var(--text-muted);
  --accent:                 var(--accent);
  --accent-foreground:      #ffffff;
  --destructive:            var(--semantic-danger);
  --border:                 var(--border-subtle);
  --input:                  var(--border-subtle);
  --ring:                   var(--accent);

  --sidebar:                var(--surface-sidebar);
  --sidebar-foreground:     var(--text-primary);
  --sidebar-primary:        var(--accent);
  --sidebar-primary-foreground: #ffffff;
  --sidebar-accent:         var(--accent);
  --sidebar-accent-foreground: #ffffff;
  --sidebar-border:         var(--border-subtle);
  --sidebar-ring:           var(--accent);

  color-scheme: light;
}
```

`.dark { ... }` block stays as today; add `color-scheme: dark;` at the bottom for native form-control / scrollbar consistency.

`.export-light { ... }` stays — it's the PDF/PNG rendering override, distinct from the global light theme.

---

## Implementation phases

### Phase 1 — light + dark with SQLite persistence

**Files**

| Action | Path |
|---|---|
| New | `src/lib/theme.ts` — `applyTheme`, `resolveMode`, `subscribeSystem`, `LOCAL_STORAGE_KEY` |
| New | `src/lib/theme.test.ts` — pure-helper coverage |
| Edit | `src/styles.css` — rewrite `:root` light tokens; add `color-scheme` per scheme |
| Edit | `src/components/app-shell/app-preferences-menu.tsx` — drop the local `applyTheme`, call `src/lib/theme.ts`, read/write through store |
| Edit | `src/components/app-shell.tsx` — drop forced `.dark`; add MutationObserver-driven Monaco-theme sync |
| Edit | `index.html` — inline pre-paint script |
| Edit | `src-tauri/src/types.rs` — `AppSettingsSnapshot.theme: ThemeMode` |
| Edit | `src-tauri/src/credentials.rs` (or wherever app-settings rows live) — persist `theme` |
| Edit | `src-tauri/src/lib.rs` — `load_app_settings` / `save_app_settings` payload extended |
| Edit | `src/lib/store/credentials.ts` (slice that owns AppSettings) — read/write theme |

**Sequence**

1. **Rust side first** so the TS layer can rely on the new field:
   1. Extend `AppSettingsSnapshot` with `theme: Option<String>` (`"system" | "light" | "dark"`).
   2. SQLite migration: add `theme TEXT` to the `app_settings` row (or however the slice is keyed — check the migration pattern used for other settings).
   3. `load_app_settings` reads it; `save_app_settings` writes it. Missing value = `None` → resolve as "system".
2. **`src/lib/theme.ts`**:
   ```ts
   export type ThemeMode = "system" | "light" | "dark";
   export const LOCAL_STORAGE_KEY = "dbunk.theme";
   export function resolveMode(mode: ThemeMode): "light" | "dark" {
     if (mode !== "system") return mode;
     return typeof window !== "undefined" &&
       window.matchMedia("(prefers-color-scheme: dark)").matches
       ? "dark" : "light";
   }
   export function applyTheme(mode: ThemeMode) {
     const resolved = resolveMode(mode);
     document.documentElement.classList.toggle("dark", resolved === "dark");
   }
   export function subscribeSystem(handler: () => void): () => void {
     const mql = window.matchMedia("(prefers-color-scheme: dark)");
     mql.addEventListener("change", handler);
     return () => mql.removeEventListener("change", handler);
   }
   ```
3. **Store wiring**:
   - On `loadAppSettings` success → read `appSettings.theme ?? "system"`, call `applyTheme`, sync to `localStorage`.
   - Menu's `onValueChange` → call store action `setTheme(mode)`, which (a) updates `appSettings`, (b) writes to localStorage, (c) calls `applyTheme(mode)`, (d) calls `save_app_settings`.
   - When mode is `"system"`, subscribe to `subscribeSystem(() => applyTheme("system"))` and cleanup on unmount / mode change.
4. **`index.html` pre-paint script** (before any `<script type="module">`):
   ```html
   <script>
     (function() {
       try {
         var t = localStorage.getItem("dbunk.theme");
         var dark = t === "dark" ||
           ((t === null || t === "system") &&
            window.matchMedia("(prefers-color-scheme: dark)").matches);
         if (dark) document.documentElement.classList.add("dark");
       } catch (e) {}
     })();
   </script>
   ```
5. **Drop `app-shell.tsx:137`** (the forced `.dark`).
6. **Monaco editor sync** in `app-shell.tsx`:
   ```tsx
   useEffect(() => {
     const sync = () => setEditorTheme(
       document.documentElement.classList.contains("dark") ? "vs-dark" : "vs",
     );
     sync();
     const observer = new MutationObserver(sync);
     observer.observe(document.documentElement, {
       attributes: true, attributeFilter: ["class"],
     });
     return () => observer.disconnect();
   }, [setEditorTheme]);
   ```
7. **Tests** (`src/lib/theme.test.ts`):
   - `resolveMode("light")` → `"light"`.
   - `resolveMode("dark")` → `"dark"`.
   - `resolveMode("system")` with `matchMedia` mocked both ways.
   - `applyTheme("dark")` adds `.dark`; `applyTheme("light")` removes it.

**Acceptance**
- New install on macOS in dark mode → app renders dark (matches OS).
- Toggle Light in menu → app turns light immediately, no reload, no flash on next reload.
- Reload with `dbunk.theme=light` in localStorage → first paint is already light (no FOUC).
- OS theme flips while menu is set to System → app follows live.
- Monaco editor follows the resolved theme in both directions.
- SQLite row carries the user's choice across app restarts; deleting localStorage falls back to SQLite on next load.

### Phase 2 — theme presets (Dracula, GitHub, Gruvbox)

Strictly additive; no refactor of Phase 1.

1. Add a `ThemePreset` type + selector in the menu.
2. New CSS blocks per preset (each redefines the same token set the `.dark` block defines). The `:root` and `.dark` blocks stay as the default preset.
3. Extend `theme.ts`:
   ```ts
   export type ThemePreset = "default" | "dracula" | "github" | "gruvbox";
   export function applyTheme(mode: ThemeMode, preset: ThemePreset = "default") {
     // ... mode logic
     if (preset === "default") document.documentElement.removeAttribute("data-theme");
     else document.documentElement.setAttribute("data-theme", preset);
   }
   ```
4. Menu becomes two pickers: Mode (System / Light / Dark) and Preset (Default / Dracula / GitHub / Gruvbox). Some presets are intrinsically dark (Dracula); the menu can gate the Mode picker accordingly.
5. `AppSettingsSnapshot.theme` evolves from a string to a struct `{ mode, preset }`. Schema migration adds the preset column.

### Phase 3 — already folded into Phase 1

The original Phase 3 (move from localStorage to AppSettings/SQLite) is absorbed into Phase 1 per the locked decision.

---

## Migration & rollout

- **Default flips from dark-only to system-resolved** for existing users. On macOS/Windows in dark mode the rendered theme is identical; on light OS the user sees light theme post-update and can flip back via the menu.
- **No data migration needed** — `dbunk.theme=null` in SQLite resolves as `"system"`.
- **No app-restart required** when changing theme — `applyTheme` runs synchronously and the MutationObserver pulls Monaco along.

## Open questions for the implementer

- The light palette above is a starting point; expect minor tweaks during implementation (especially `--surface-row-hover` and `--accent-overlay` contrast against `--surface-window`). Eyeball-test on the data grid, schema map, and CLI tab before committing.
- The pre-paint inline `<script>` in `index.html` is the simplest FOUC fix. If `index.html` is generated by a framework or doesn't allow inline scripts (CSP), the same logic can run at the top of the main entry module — accept the cost of a first-paint flash on very slow loads.
- The MutationObserver approach to syncing Monaco assumes only one place mutates `.dark` on `<html>` (the menu / pre-paint script). If that ever stops being true, switch to a custom event or a zustand subscribe.

## Out of scope for this plan

- Per-component theme overrides (e.g. "always-dark editor inside a light app"). Not requested; would require a separate scoping mechanism.
- Theme transitions / animations between modes (just flip the class).
- User-defined themes / theme imports. Possible later if Phase 2 lands.

## Estimate

- Phase 1 (light + dark + SQLite): ~half-day.
- Phase 2 (3 presets): ~half-day per preset, mostly visual tuning.
