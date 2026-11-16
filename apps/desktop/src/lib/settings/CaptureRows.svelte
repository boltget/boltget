<!--
  Settings, Browser: the capture rules the extension applies
  (docs/BROWSER-INTEGRATION.md, "Capture rules"). Each row applies as it
  changes, like the rest of Settings: switches and checkboxes at once, typed
  values on Enter (Ctrl+Enter in a site list) or when the field is left,
  after a check that says what is wrong under the field. The extension
  takes the saved rules with its next `hello`. "Forget private-window
  downloads" sits by "Private windows" but is the app's own setting, not a
  rule: the extension never sees it, and Restore defaults leaves it.
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import Checkbox from "@boltget/ui/components/Checkbox.svelte";
  import Switch from "@boltget/ui/components/Switch.svelte";
  import TextField from "@boltget/ui/components/TextField.svelte";
  import { t } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { app } from "../app.svelte";
  import type { CapturePatch, CaptureSettings, SettingsPatch } from "../bindings";
  import {
    DEFAULT_CAPTURE,
    formatMinSize,
    groupState,
    otherTypes,
    parseMinSize,
    parseSites,
    sameCapture,
    setGroup,
    TYPE_GROUPS,
    withOtherTypes,
    type TypeGroup,
  } from "./capture";
  import type { RowId } from "./registry";
  import SettingRow from "./SettingRow.svelte";

  interface Props {
    capture: CaptureSettings;
    /** "Forget private-window downloads" (an app setting, not a rule). */
    forgetPrivate: boolean;
    /** The rows on show (the section's, or what a search found). */
    shown: Set<RowId>;
    highlighted: RowId | null;
    problems: Record<string, string | null>;
    problem: (field: string, text: string | null) => void;
    save: (field: string, patch: SettingsPatch) => Promise<boolean>;
  }

  let { capture, forgetPrivate, shown, highlighted, problems, problem, save }: Props = $props();

  type Draft = "minSize" | "other" | "never" | "always";
  let editing = $state<Draft | null>(null);
  let minSize = $state("");
  let other = $state("");
  let never = $state("");
  let always = $state("");
  /** The site list the problem under the row is about. */
  let badList = $state<"never" | "always" | null>(null);

  // Drafts follow the saved rules, except the field being typed in.
  $effect(() => {
    const current = capture;
    untrack(() => syncDrafts(current));
  });

  // A draft that was refused stays, with its problem, until it is fixed or
  // Esc puts the saved value back.
  function syncDrafts(current: CaptureSettings) {
    if (editing !== "minSize" && !problems.capture_size) minSize = formatMinSize(current.minSize);
    if (editing !== "other" && !problems.capture_types) {
      other = otherTypes(current.extensions).join(" ");
    }
    if (editing !== "never" && badList !== "never") never = current.neverSites.join("\n");
    if (editing !== "always" && badList !== "always") always = current.alwaysSites.join("\n");
  }

  function clearProblems() {
    badList = null;
    for (const field of ["capture_size", "capture_types", "capture_sites"]) problem(field, null);
  }

  const change = (field: string, patch: CapturePatch) => save(field, { capture: patch });

  function onswitch(field: string, patch: (on: boolean) => CapturePatch) {
    return (event: Event) => {
      const input = event.currentTarget as HTMLInputElement;
      const on = input.checked;
      void change(field, patch(on)).then((saved) => {
        if (!saved) input.checked = !on;
      });
    };
  }

  /** Esc puts the saved value back; `enter` decides whether Enter commits. */
  function keys(commit: () => void, enter: (e: KeyboardEvent) => boolean) {
    return (event: KeyboardEvent) => {
      if (event.key === "Enter" && enter(event)) {
        event.preventDefault();
        commit();
      } else if (event.key === "Escape") {
        // Puts the saved value back, rather than leaving Settings.
        event.stopPropagation();
        editing = null;
        clearProblems();
        syncDrafts(capture);
        (event.currentTarget as HTMLElement).blur();
      }
    };
  }
  const plainEnter = () => true;
  const ctrlEnter = (e: KeyboardEvent) => e.ctrlKey;

  function commitMinSize() {
    editing = null;
    const parsed = parseMinSize(minSize);
    if (!parsed.ok) return problem("capture_size", parsed.problem);
    problem("capture_size", null);
    if (parsed.value === capture.minSize) {
      minSize = formatMinSize(capture.minSize);
      return;
    }
    void change("capture_size", { minSize: parsed.value });
  }

  function toggleGroup(group: TypeGroup) {
    return (event: Event) => {
      const input = event.currentTarget as HTMLInputElement;
      const before = groupState(capture.extensions, group);
      const on = input.checked;
      void change("capture_types", { extensions: setGroup(capture.extensions, group, on) }).then(
        (saved) => {
          if (saved) return;
          input.checked = before === "on";
          input.indeterminate = before === "mixed";
        },
      );
    };
  }

  function commitOther() {
    editing = null;
    const parsed = withOtherTypes(capture.extensions, other);
    if (!parsed.ok) return problem("capture_types", parsed.problem);
    problem("capture_types", null);
    if (parsed.value.join(" ") === capture.extensions.join(" ")) {
      other = otherTypes(capture.extensions).join(" ");
      return;
    }
    void change("capture_types", { extensions: parsed.value });
  }

  function commitSites(which: "never" | "always") {
    editing = null;
    const parsed = parseSites(which === "never" ? never : always);
    if (!parsed.ok) {
      badList = which;
      return problem("capture_sites", parsed.problem);
    }
    badList = null;
    problem("capture_sites", null);
    const current = which === "never" ? capture.neverSites : capture.alwaysSites;
    if (parsed.value.join("\n") === current.join("\n")) {
      syncDrafts(capture);
      return;
    }
    void change(
      "capture_sites",
      which === "never" ? { neverSites: parsed.value } : { alwaysSites: parsed.value },
    );
  }

  const isDefault = $derived(sameCapture(capture, DEFAULT_CAPTURE));

  async function restore() {
    const before: CapturePatch = { ...capture };
    editing = null;
    if (!(await change("capture_defaults", { ...DEFAULT_CAPTURE }))) return;
    clearProblems();
    syncDrafts(capture);
    app.toast(t("settings.capture_defaults.restored"), {
      label: t("action.undo"),
      run: () => void change("capture_defaults", before),
    });
  }

  const describedBy = (id: string) =>
    [`setting-${id}-description`, problems[id] ? `setting-${id}-problem` : ""]
      .filter(Boolean)
      .join(" ");
</script>

{#if shown.has("capture")}
  <SettingRow
    id="capture"
    label={t("settings.capture")}
    description={t("settings.capture.description")}
    target={highlighted === "capture"}
    problem={problems.capture}
  >
    {#snippet control()}
      <Switch
        checked={capture.enabled}
        aria-labelledby="setting-capture"
        aria-describedby={describedBy("capture")}
        onchange={onswitch("capture", (enabled) => ({ enabled }))}
      />
    {/snippet}
  </SettingRow>
{/if}

{#if shown.has("capture_size")}
  <SettingRow
    id="capture_size"
    label={t("settings.capture_size")}
    description={t("settings.capture_size.description")}
    target={highlighted === "capture_size"}
    problem={problems.capture_size}
  >
    {#snippet control()}
      <span class="size">
        <TextField
          bind:value={minSize}
          inputmode="decimal"
          unit={t("settings.capture_size.unit")}
          aria-label={t("settings.capture_size.input")}
          aria-describedby={describedBy("capture_size")}
          invalid={!!problems.capture_size}
          onfocus={() => (editing = "minSize")}
          onblur={commitMinSize}
          onkeydown={keys(commitMinSize, plainEnter)}
          oninput={() => problem("capture_size", null)}
        />
      </span>
    {/snippet}
  </SettingRow>
{/if}

{#if shown.has("capture_types")}
  <SettingRow
    id="capture_types"
    label={t("settings.capture_types")}
    description={t("settings.capture_types.description")}
    target={highlighted === "capture_types"}
    problem={problems.capture_types}
  >
    <ul class="groups" aria-labelledby="setting-capture_types">
      {#each TYPE_GROUPS as group (group.id)}
        {@const state = groupState(capture.extensions, group)}
        <li class="group">
          <Checkbox
            checked={state === "on"}
            indeterminate={state === "mixed"}
            aria-describedby="capture-group-{group.id}"
            onchange={toggleGroup(group)}
          >
            {t(group.label)}
          </Checkbox>
          <span class="types" id="capture-group-{group.id}">{group.extensions.join(" ")}</span>
        </li>
      {/each}
    </ul>
    <label class="field" for="capture-other">
      <span class="field-label">{t("settings.capture_types.other")}</span>
      <TextField
        id="capture-other"
        mono
        bind:value={other}
        placeholder={t("settings.capture_types.other.placeholder")}
        aria-describedby={problems.capture_types ? "setting-capture_types-problem" : undefined}
        invalid={!!problems.capture_types}
        onfocus={() => (editing = "other")}
        onblur={commitOther}
        onkeydown={keys(commitOther, plainEnter)}
        oninput={() => problem("capture_types", null)}
      />
    </label>
  </SettingRow>
{/if}

{#if shown.has("capture_sites")}
  <SettingRow
    id="capture_sites"
    label={t("settings.capture_sites")}
    description={t("settings.capture_sites.description")}
    target={highlighted === "capture_sites"}
    problem={problems.capture_sites}
  >
    <div class="fields">
      <label class="field" for="capture-never">
        <span class="field-label">{t("settings.capture_sites.never")}</span>
        <textarea
          id="capture-never"
          rows="4"
          spellcheck="false"
          autocomplete="off"
          bind:value={never}
          placeholder={t("settings.capture_sites.placeholder")}
          aria-describedby="setting-capture_sites-description {problems.capture_sites
            ? 'setting-capture_sites-problem'
            : ''}"
          aria-invalid={(!!problems.capture_sites && badList !== "always") || undefined}
          onfocus={() => (editing = "never")}
          onblur={() => commitSites("never")}
          onkeydown={keys(() => commitSites("never"), ctrlEnter)}
          oninput={() => problem("capture_sites", null)}></textarea>
      </label>
      <label class="field" for="capture-always">
        <span class="field-label">{t("settings.capture_sites.always")}</span>
        <textarea
          id="capture-always"
          rows="4"
          spellcheck="false"
          autocomplete="off"
          bind:value={always}
          placeholder={t("settings.capture_sites.placeholder")}
          aria-describedby="capture-always-hint {problems.capture_sites
            ? 'setting-capture_sites-problem'
            : ''}"
          aria-invalid={(!!problems.capture_sites && badList !== "never") || undefined}
          onfocus={() => (editing = "always")}
          onblur={() => commitSites("always")}
          onkeydown={keys(() => commitSites("always"), ctrlEnter)}
          oninput={() => problem("capture_sites", null)}></textarea>
        <span class="field-hint" id="capture-always-hint"
          >{t("settings.capture_sites.always.hint")}</span
        >
      </label>
    </div>
  </SettingRow>
{/if}

{#if shown.has("capture_private")}
  <SettingRow
    id="capture_private"
    label={t("settings.capture_private")}
    description={t("settings.capture_private.description")}
    target={highlighted === "capture_private"}
    problem={problems.capture_private}
  >
    {#snippet control()}
      <Switch
        checked={capture.privateWindows}
        aria-labelledby="setting-capture_private"
        aria-describedby={describedBy("capture_private")}
        onchange={onswitch("capture_private", (privateWindows) => ({ privateWindows }))}
      />
    {/snippet}
  </SettingRow>
{/if}

{#if shown.has("forget_private")}
  <SettingRow
    id="forget_private"
    label={t("settings.forget_private")}
    description={t("settings.forget_private.description")}
    target={highlighted === "forget_private"}
    problem={problems.forget_private}
  >
    {#snippet control()}
      <Switch
        checked={forgetPrivate}
        aria-labelledby="setting-forget_private"
        aria-describedby={describedBy("forget_private")}
        onchange={(event: Event) => {
          const input = event.currentTarget as HTMLInputElement;
          const on = input.checked;
          void save("forget_private", { forgetPrivate: on }).then((saved) => {
            if (!saved) input.checked = !on;
          });
        }}
      />
    {/snippet}
  </SettingRow>
{/if}

{#if shown.has("capture_defaults")}
  <SettingRow
    id="capture_defaults"
    label={t("settings.capture_defaults")}
    description={t("settings.capture_defaults.description")}
    target={highlighted === "capture_defaults"}
    problem={problems.capture_defaults}
  >
    {#snippet control()}
      <Button
        disabled={isDefault}
        aria-describedby={describedBy("capture_defaults")}
        onclick={() => void restore()}
      >
        {t("settings.capture_defaults.restore")}
      </Button>
    {/snippet}
  </SettingRow>
{/if}

<style>
  .size {
    display: inline-flex;
    width: 120px;
  }
  .size :global(.text-field) {
    width: 100%;
  }

  /* File types: two columns of groups, each with its types in mono. */
  .groups {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-1) var(--space-6);
    margin: 0 0 var(--space-3);
    padding: var(--space-3) 0 0;
    list-style: none;
    border-top: 1px solid var(--color-hairline);
  }
  .group {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: var(--space-1) 0;
  }
  .types {
    padding-left: calc(16px + var(--space-2));
    color: var(--color-text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }

  .fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .field :global(.text-field) {
    width: 100%;
  }
  .field-label {
    color: var(--color-text-muted);
    font-size: var(--text-12);
  }
  .field-hint {
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }

  /* A site list: the text field's look, several lines high. */
  textarea {
    width: 100%;
    min-height: calc(var(--control-height) * 3);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline-strong);
    border-bottom-color: var(--color-control-stroke);
    border-radius: var(--radius-control);
    background: var(--color-field);
    color: var(--color-text);
    font-family: var(--font-mono);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    resize: vertical;
    user-select: text;
  }
  textarea::placeholder {
    color: var(--color-text-subtle);
  }
  textarea:focus-visible {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: 0 0 0 1px var(--color-accent);
  }
  textarea[aria-invalid="true"] {
    border-color: var(--color-error);
  }

  @media (max-width: 719px) {
    .groups,
    .fields {
      grid-template-columns: 1fr;
    }
  }
  @media (forced-colors: active) {
    textarea {
      border-color: ButtonText;
    }
  }
</style>
