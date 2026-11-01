// ESLint for @boltget/desktop (flat config). packages/ui uses the same rules.
import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import ts from "typescript-eslint";

export default defineConfig(
  // src/lib/bindings.ts is generated from Rust (src-tauri/src/ipc.rs).
  { ignores: ["node_modules/", "dist/", "src-tauri/", "src/lib/bindings.ts"] },
  js.configs.recommended,
  ts.configs.recommended,
  svelte.configs.recommended,
  { languageOptions: { globals: { ...globals.browser } } },
  {
    files: ["*.config.js", "*.config.ts", "scripts/**"],
    languageOptions: { globals: { ...globals.node } },
  },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: { parserOptions: { parser: ts.parser } },
  },
);
