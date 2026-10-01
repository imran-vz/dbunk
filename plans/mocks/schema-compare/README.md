# Plan 022 comparison mocks

[Published private brief and mocks](https://dbunk-schema-compare-plan-022.imran-vz.chatgpt.site) ·
[Completed plan](https://github.com/imran-vz/dbunk/blob/db2dae24c504248d62f15f772bf82e4c8d1f5ff2/plans/022-postgres-schema-comparison-activation.md)

**Selected: A, Object inspector**, chosen by Imran on 2026-09-14. The unused
matrix and stacked-review layouts have been removed, including their styles,
controls and rendering paths. The brief reflects the selected design.

The mock uses the actual workbench shell metrics, installed Tabler icons,
bundled JetBrains Mono and a true-black comparison surface. Fixture values are
invented. App-shell and endpoint controls are static illustrations; mock/plan
navigation, states, object selection and field inspection are interactive.

Planning verification on 2026-09-14: `pnpm format`, `pnpm lint`,
`pnpm typecheck`, JavaScript syntax and local documentation links passed.
The browser rendered the layouts and planning brief; the narrow 390px check
had no page-level horizontal overflow, and captured JavaScript error logs
were empty. These checks validate the planning artifact, not native comparison
or WebView memory behavior. No product source was edited.

After cleanup, format/lint/typecheck and JavaScript syntax passed again.
Browser checks covered the retained Object inspector, field-value selection,
the current plan and a 390px viewport without page overflow or JavaScript errors.

Publishing is isolated from dbunk's repository and build channels. Reuse this
private Site for revisions, do not create another:

- Site project: `appgprj_6aa7afd2bcd8819197c592608aed0a23`.
- Local publishing checkout: `/tmp/dbunk-plan-022-site`.
- Canonical artifact source: `index.html` beside this file.
- Deployed source: `fa37e02f9d7b83abbf24d1684dc92b7389dca483`.

If the temporary checkout is gone, recreate it from the canonical artifact,
restore the same project ID in its isolated hosting manifest and obtain a fresh
source credential. Never place Site hosting configuration at dbunk's repo root.
