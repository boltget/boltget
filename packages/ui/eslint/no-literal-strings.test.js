import { RuleTester } from "eslint";
import svelte from "eslint-plugin-svelte";
import ts from "typescript-eslint";
import { describe, it } from "vitest";
import { isVisibleText, noLiteralStrings } from "./no-literal-strings.js";

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const svelteParser = svelte.configs.recommended.find((c) => c.languageOptions?.parser)
  ?.languageOptions.parser;

const markup = new RuleTester({
  languageOptions: { parser: svelteParser, parserOptions: { parser: ts.parser } },
});
const script = new RuleTester({ languageOptions: { parser: ts.parser } });

const err = { messageId: "literal" };

describe("isVisibleText", () => {
  it("treats words as text and symbols, numbers and keys as not", ({ expect }) => {
    expect(isVisibleText("Pause all")).toBe(true);
    expect(isVisibleText("  ·  ")).toBe(false);
    expect(isVisibleText("2×")).toBe(false);
    expect(isVisibleText("command.pause_all")).toBe(false);
    expect(isVisibleText("Ünïcode")).toBe(true);
  });
});

markup.run("no-literal-strings (Svelte)", noLiteralStrings, {
  valid: [
    { filename: "a.svelte", code: `<p>{t("empty.title")}</p>` },
    { filename: "a.svelte", code: `<span>·</span><b> / </b><i>{n}×</i>` },
    {
      filename: "a.svelte",
      code: `<button aria-label={t("caption.close")} class="close x"></button>`,
    },
    { filename: "a.svelte", code: `<div role="listbox" id="list" data-kind="queue"></div>` },
    { filename: "a.svelte", code: `<p>{state === "running" ? t("a.b") : t("a.c")}</p>` },
    { filename: "a.svelte", code: `<Row label="command.add" />` },
    { filename: "a.svelte", code: `<style>.a::after { content: "Hello"; }</style>` },
    {
      filename: "a.svelte",
      code: `<script lang="ts">const kind = "running"; if (kind === "paused") {}</script>`,
    },
  ],
  invalid: [
    { filename: "a.svelte", code: `<p>Paste a link</p>`, errors: [err] },
    { filename: "a.svelte", code: `<p>{busy ? "Saving" : label}</p>`, errors: [err] },
    { filename: "a.svelte", code: `<button aria-label="Close"></button>`, errors: [err] },
    { filename: "a.svelte", code: `<input placeholder="Search" />`, errors: [err] },
    { filename: "a.svelte", code: `<span title={on ? "On" : "Off"}></span>`, errors: [err, err] },
    { filename: "a.svelte", code: `<Button label={\`Add \${n}\`} />`, errors: [err] },
    {
      filename: "a.svelte",
      code: `<script lang="ts">const item = { label: "Pause" };</script>`,
      errors: [err],
    },
  ],
});

script.run("no-literal-strings (TS)", noLiteralStrings, {
  valid: [
    { code: `const a = { label: "command.add", id: "add" };` },
    { code: `el.textContent = t("view.all");` },
    { code: `el.setAttribute("role", "status");` },
    { code: `const kind = "running";` },
  ],
  invalid: [
    { code: `el.textContent = "Done";`, errors: [err] },
    { code: `el.setAttribute("aria-label", "Close");`, errors: [err] },
    { code: `const a = { title: ok ? "Saved" : "Failed" };`, errors: [err, err] },
    { code: `const a = { message: "Removed " + name };`, errors: [err] },
  ],
});
