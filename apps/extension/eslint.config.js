// ESLint for @boltget/extension (flat config), the same rules as the app.
import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import ts from "typescript-eslint";
import { catalogRule } from "@boltget/ui/eslint";

export default defineConfig(
  { ignores: ["node_modules/", ".output/", ".wxt/", "test-results/", "playwright-report/"] },
  js.configs.recommended,
  ts.configs.recommended,
  svelte.configs.recommended,
  { languageOptions: { globals: { ...globals.browser, ...globals.webextensions } } },
  {
    files: ["*.config.js", "*.config.ts", "scripts/**", "e2e/**"],
    languageOptions: { globals: { ...globals.node } },
  },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: { parserOptions: { parser: ts.parser } },
  },
  // User-visible text comes from the string catalog (lib/en.json).
  catalogRule(["entrypoints/**/*.svelte", "entrypoints/**/*.ts", "lib/**/*.svelte", "lib/**/*.ts"]),
);
