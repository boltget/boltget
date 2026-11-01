// ESLint for @boltget/desktop (flat config). packages/ui uses the same rules.
import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import ts from "typescript-eslint";

export default defineConfig(
  { ignores: ["node_modules/", "dist/", "src-tauri/"] },
  js.configs.recommended,
  ts.configs.recommended,
  svelte.configs.recommended,
  { languageOptions: { globals: { ...globals.browser } } },
  { files: ["*.config.js", "*.config.ts"], languageOptions: { globals: { ...globals.node } } },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: { parserOptions: { parser: ts.parser } },
  },
);
