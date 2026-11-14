<!--
  A queue's settings, or a new queue (APP-DESIGN, "Queues as built"): its
  name, how many downloads it runs at once, its daily start and stop times,
  and what to do when it finishes. A modal <dialog> inside the window, like
  the add sheet: Esc or Cancel closes it, Enter saves. The page checks what
  it can first and says what is wrong under the field; the engine checks
  again (a name taken) and answers with the same words.
-->
<script lang="ts">
  import { trapTab } from "../focus";
  import Button from "@boltget/ui/components/Button.svelte";
  import Select from "@boltget/ui/components/Select.svelte";
  import TextField from "@boltget/ui/components/TextField.svelte";
  import { isPlainMessageKey, t } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { app } from "../app.svelte";
  import type { FinishAction, QueueInfo, QueuePatch } from "../bindings";
  import { downloads } from "../engine";
  import { queueView } from "../list/model";
  import { parseTime, timeValue } from "../list/queue";

  type Field = "name" | "limit" | "times";

  let dialog: HTMLDialogElement | undefined = $state();
  let nameField: HTMLInputElement | undefined = $state();
  let editing = $state<QueueInfo | null>(null);
  let isNew = $state(false);
  let name = $state("");
  let limit = $state("3");
  let start = $state("");
  let stop = $state("");
  let finish = $state<FinishAction>("nothing");
  let problems = $state<Partial<Record<Field, string>>>({});
  let saving = $state(false);

  const FINISH: { value: FinishAction; label: string }[] = [
    { value: "nothing", label: t("queue.finish.nothing") },
    { value: "sleep", label: t("queue.finish.sleep") },
    { value: "shut_down", label: t("queue.finish.shut_down") },
  ];

  $effect(() => {
    const request = app.queueEditing;
    untrack(() => {
      if (request && !dialog?.open) show(request.id);
      else if (!request && dialog?.open) dialog.close();
    });
  });

  function show(id: number | null) {
    const q = id === null ? null : (downloads.queue(id) ?? null);
    editing = q;
    isNew = q === null;
    name = q ? app.queueName(q) : "";
    limit = String(q?.maxConcurrent ?? 3);
    start = timeValue(q?.startTime ?? null);
    stop = timeValue(q?.stopTime ?? null);
    finish = q?.onFinish ?? "nothing";
    problems = {};
    saving = false;
    queueMicrotask(() => {
      dialog?.showModal();
      nameField?.focus();
      nameField?.select();
    });
  }

  function close() {
    app.queueEditing = null;
  }

  function fieldOf(key: string): Field {
    if (key.startsWith("error.queue.name")) return "name";
    if (key === "error.queue.limit") return "limit";
    return "times";
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    const n = Number(limit.trim());
    const startMin = start ? parseTime(start) : null;
    const stopMin = stop ? parseTime(stop) : null;
    const next: Partial<Record<Field, string>> = {};
    if (!name.trim()) next.name = t("error.queue.name_empty");
    else if (name.trim().length > 40) next.name = t("error.queue.name_too_long");
    if (!Number.isInteger(n) || n < 1 || n > 32) next.limit = t("error.queue.limit");
    if (startMin !== null && startMin === stopMin) next.times = t("error.queue.window");
    problems = next;
    if (Object.keys(next).length) return;

    saving = true;
    let target = editing;
    if (isNew) {
      const created = await downloads.createQueue(name.trim());
      if (created.status === "error") {
        problems = { [fieldOf(created.error.key)]: message(created.error.key) };
        saving = false;
        return;
      }
      target = created.data;
    }
    if (!target) return close();
    const patch: QueuePatch = {};
    // The default queue keeps its own name until the user gives it one.
    if (!isNew && name.trim() !== app.queueName(target)) patch.name = name.trim();
    if (n !== target.maxConcurrent) patch.maxConcurrent = n;
    if (startMin !== target.startTime || stopMin !== target.stopTime) {
      patch.times = { start: startMin, stop: stopMin };
    }
    if (finish !== target.onFinish) patch.onFinish = finish;
    if (Object.keys(patch).length) {
      const result = await downloads.updateQueue(target.id, patch);
      if (result.status === "error") {
        problems = { [fieldOf(result.error.key)]: message(result.error.key) };
        saving = false;
        return;
      }
    }
    saving = false;
    if (isNew) app.showQueue(target.id);
    close();
  }

  function message(key: string): string {
    return isPlainMessageKey(key) ? t(key) : t("error.unknown");
  }

  function remove() {
    const q = editing;
    if (!q || q.isDefault) return;
    const shown = app.queueName(q);
    close();
    void downloads.deleteQueue(q.id).then((r) => {
      if (r.status !== "ok") return;
      if (app.view === queueView(q.id)) app.showQueue(1);
      app.toast(t("queue.deleted", { name: shown, main: t("queue.main") }));
    });
  }
</script>

<dialog
  bind:this={dialog}
  use:trapTab
  class="dialog"
  aria-labelledby="queue-title"
  onclose={() => {
    if (app.queueEditing) close();
  }}
>
  {#if app.queueEditing}
    <form onsubmit={save} novalidate>
      <h2 id="queue-title" class="title">{isNew ? t("queue.new") : t("queue.settings")}</h2>

      <div class="field">
        <label for="queue-name">{t("queue.name")}</label>
        <TextField
          id="queue-name"
          bind:ref={nameField}
          bind:value={name}
          maxlength={40}
          invalid={!!problems.name}
          aria-describedby={problems.name ? "queue-name-problem" : undefined}
          oninput={() => (problems = { ...problems, name: undefined })}
        />
        {#if problems.name}
          <p class="problem" id="queue-name-problem" role="alert">{problems.name}</p>
        {/if}
      </div>

      <div class="field">
        <label for="queue-limit">{t("queue.limit")}</label>
        <p class="description" id="queue-limit-description">{t("queue.limit.description")}</p>
        <span class="narrow">
          <TextField
            id="queue-limit"
            bind:value={limit}
            inputmode="numeric"
            invalid={!!problems.limit}
            aria-describedby={problems.limit
              ? "queue-limit-description queue-limit-problem"
              : "queue-limit-description"}
            oninput={() => (problems = { ...problems, limit: undefined })}
          />
        </span>
        {#if problems.limit}
          <p class="problem" id="queue-limit-problem" role="alert">{problems.limit}</p>
        {/if}
      </div>

      <fieldset class="field">
        <legend>{t("queue.times")}</legend>
        <p class="description">{t("queue.times.description")}</p>
        <div class="times">
          <label class="time">
            <span>{t("queue.start_time")}</span>
            <TextField
              type="time"
              bind:value={start}
              invalid={!!problems.times}
              oninput={() => (problems = { ...problems, times: undefined })}
            />
          </label>
          <label class="time">
            <span>{t("queue.stop_time")}</span>
            <TextField
              type="time"
              bind:value={stop}
              invalid={!!problems.times}
              oninput={() => (problems = { ...problems, times: undefined })}
            />
          </label>
        </div>
        {#if problems.times}
          <p class="problem" role="alert">{problems.times}</p>
        {/if}
      </fieldset>

      <div class="field">
        <label for="queue-finish">{t("queue.finish")}</label>
        <p class="description" id="queue-finish-description">{t("queue.finish.description")}</p>
        <Select
          id="queue-finish"
          bind:value={finish}
          options={FINISH}
          aria-describedby="queue-finish-description"
        />
      </div>

      <footer class="foot">
        {#if editing && !editing.isDefault}
          <span class="left">
            <Button variant="ghost" onclick={remove}>{t("queue.delete")}</Button>
          </span>
        {/if}
        <Button onclick={close}>{t("action.cancel")}</Button>
        <Button variant="primary" type="submit" disabled={saving}>
          {isNew ? t("queue.create") : t("queue.save")}
        </Button>
      </footer>
      {#if editing && !editing.isDefault}
        <p class="note">{t("queue.delete.note", { main: t("queue.main") })}</p>
      {/if}
    </form>
  {/if}
</dialog>

<style>
  .dialog {
    width: min(460px, calc(100vw - 48px));
    max-height: calc(100vh - 96px);
    margin: 72px auto auto;
    padding: var(--space-6);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-raised);
    color: var(--color-text);
    box-shadow: var(--elevation-floating);
    overflow-y: auto;
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
      transform: translateY(-8px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dialog[open] {
      animation: none;
    }
  }
  .title {
    margin: 0 0 var(--space-4);
    font-family: var(--font-display);
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
    letter-spacing: var(--tracking-15);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0 0 var(--space-4);
    padding: 0;
    border: 0;
    min-width: 0;
  }
  label,
  legend {
    padding: 0;
    font-weight: var(--weight-medium);
  }
  .description,
  .note {
    margin: 0 0 var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-secondary);
  }
  .note {
    margin: var(--space-3) 0 0;
  }
  .narrow {
    width: 96px;
  }
  .times {
    display: flex;
    gap: var(--space-4);
  }
  .time {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-weight: var(--weight-regular);
    color: var(--color-text-muted);
    font-size: var(--text-12);
  }
  .problem {
    margin: 0;
    color: var(--color-error);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-6);
  }
  .left {
    margin-right: auto;
  }
</style>
