/**
 * Font stacks — single source of truth for surfaces that can't read
 * CSS custom properties (Monaco). Keep in sync with `--font-mono` in
 * src/styles.css. JetBrains Mono is bundled and must be first so mono
 * surfaces render identically on every platform
 * (DESIGN-SYSTEM.md §2.1).
 */
export const MONO_FONT_FAMILY =
  '"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, Menlo, Consolas, monospace';
