<!--
  "Start later…" for downloads already in the list (APP-DESIGN, "Queues as
  built"): one date and time, local. At that time they start, whatever
  their queue does; until then they wait, and the Scheduled view lists
  them. A time already passed is refused here, so nothing starts by
  surprise.
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import TextField from "@boltget/ui/components/TextField.svelte";
  import { t } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { app } from "../app.svelte";
  import { downloads } from "../engine";
  import { displayName } from "../format";
  import { dateTimeValue, nextHour, parseDateTime } from "../list/queue";

  let dialog: HTMLDialogElement | undefined = $state();
  let field: HTMLInputElement | undefined = $state();
  let when = $state("");
  let problem = $state<string | null>(null);

  const ids = $derived(app.startingLater ?? []);
  const name = $derived.by(() => {
    const d = ids[0] === undefined ? undefined : downloads.get(ids[0]);
    return d ? displayName(d.fileName, d.url) : "";
  });

  $effect(() => {
    const request = app.startingLater;
    untrack(() => {
      if (request && !dialog?.open) {
        const first = downloads.get(request[0] as number);
        when = dateTimeValue(first?.startAt ?? nextHour(Date.now()));
        problem = null;
        queueMicrotask(() => {
          dialog?.showModal();
          field?.focus();
        });
      } else if (!request && dialog?.open) dialog.close();
    });
  });

  function close() {
    app.startingLater = null;
  }

  function save(event: SubmitEvent) {
    event.preventDefault();
    const at = parseDateTime(when);
    if (at === null || at <= Date.now()) {
      problem = t("later.past");
      return;
    }
    for (const id of ids) void downloads.setStartAt(id, at);
    close();
  }
</script>

<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="later-title"
  aria-describedby="later-body"
  onclose={() => {
    if (app.startingLater) close();
  }}
>
  {#if app.startingLater}
    <form onsubmit={save} novalidate>
      <h2 id="later-title" class="title">{t("later.title")}</h2>
      <p id="later-body" class="body">{t("later.body", { count: ids.length, name })}</p>
      <label class="field">
        <span>{t("later.when")}</span>
        <TextField
          type="datetime-local"
          bind:ref={field}
          bind:value={when}
          invalid={!!problem}
          oninput={() => (problem = null)}
        />
      </label>
      {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
      <div class="buttons">
        <Button onclick={close}>{t("action.cancel")}</Button>
        <Button variant="primary" type="submit">{t("later.set")}</Button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  .dialog {
    width: min(400px, calc(100vw - 48px));
    padding: var(--space-6);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-raised);
    color: var(--color-text);
    box-shadow: var(--elevation-floating);
  }
  .dialog[open] {
    animation: enter var(--duration-pane) var(--ease-enter);
  }
  .dialog::backdrop {
    background: var(--color-scrim);
  }
  @keyframes enter {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dialog[open] {
      animation: none;
    }
  }
  .title {
    margin: 0;
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
  }
  .body {
    margin: var(--space-2) 0 var(--space-4);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-weight: var(--weight-medium);
  }
  .problem {
    margin: var(--space-1) 0 0;
    color: var(--color-error);
    font-size: var(--text-12);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-6);
  }
</style>
