/**
 * Configure Monaco Editor to load from the local bundle instead of CDN.
 *
 * This ensures the SQL editor works fully offline in the Tauri desktop
 * app. Import this module once at app startup (before any Monaco editor
 * component mounts).
 *
 * We import only the core editor (no TypeScript/CSS/HTML/JSON language
 * services) to keep the bundle small. SQL highlighting is provided by
 * Monaco's built-in monarch tokenizer which doesn't need a language worker.
 */

import { loader } from "@monaco-editor/react";
// @ts-expect-error — deep ESM import has no standalone type declarations
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
// Register the base editor worker. Language-specific workers (TS, CSS,
// HTML, JSON) are not needed for SQL editing.
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

self.MonacoEnvironment = {
  getWorker() {
    return new editorWorker();
  },
};

// Tell @monaco-editor/react to use our local Monaco instance instead
// of fetching from cdn.jsdelivr.net.
loader.config({ monaco });
