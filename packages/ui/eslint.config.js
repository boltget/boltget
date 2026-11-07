// ESLint for @boltget/ui (flat config). apps/desktop uses the same rules.
import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import ts from "typescript-eslint";
import { catalogRule } from "./eslint/no-literal-strings.js";

export default defineConfig(
  { ignores: ["node_modules/", "dist/", "src/messages/keys.ts"] },
  js.configs.recommended,
  ts.configs.recommended,
  svelte.configs.recommended,
  { languageOptions: { globals: { ...globals.browser } } },
  { files: ["scripts/**/*.mjs", "*.config.js"], languageOptions: { globals: { ...globals.node } } },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: { parserOptions: { parser: ts.parser } },
  },
  // User-visible text comes from the string catalog (packages/ui/eslint).
  catalogRule(["src/**/*.svelte", "src/**/*.ts"]),
);
