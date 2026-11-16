<!--
  The confirmations the app asks for: deleting files from disk, which cannot
  be undone (DESIGN-PRINCIPLES #6, #7), and resuming a download with a link
  the browser handed over. A native <dialog> shown modally inside the window:
  it traps focus and Esc cancels. For deleting, Cancel has focus first so
  Enter never deletes by accident; for a primary question, the recommended
  answer has focus.
-->
<script lang="ts">
  import { trapTab } from "./focus";
  import Button from "@boltget/ui/components/Button.svelte";
  import { t } from "@boltget/ui/messages";
  import { app } from "./app.svelte";

  let dialog: HTMLDialogElement | undefined = $state();
  let cancel: HTMLButtonElement | undefined = $state();
  let ok: HTMLButtonElement | undefined = $state();
  const request = $derived(app.confirmation);

  $effect(() => {
    if (!dialog) return;
    if (request && !dialog.open) {
      dialog.showModal();
      (request.tone === "primary" ? ok : cancel)?.focus();
    } else if (!request && dialog.open) dialog.close();
  });

  function confirm() {
    const r = app.confirmation;
    app.confirmation = null;
    r?.run();
  }

  /** The other button, Esc, or the dialog closing while it still asks. */
  function dismiss() {
    const r = app.confirmation;
    if (!r) return;
    app.confirmation = null;
    r.onCancel?.();
  }
</script>

<dialog
  bind:this={dialog}
  use:trapTab
  class="dialog"
  aria-labelledby="confirm-title"
  aria-describedby="confirm-body"
  onclose={dismiss}
>
  {#if request}
    <h2 id="confirm-title" class="title">{request.title}</h2>
    <p id="confirm-body" class="body">{request.body}</p>
    <div class="buttons">
      <Button bind:ref={cancel} onclick={dismiss}>{request.cancel ?? t("action.cancel")}</Button>
      <Button bind:ref={ok} variant={request.tone ?? "danger"} onclick={confirm}
        >{request.confirm}</Button
      >
    </div>
  {/if}
</dialog>

<style>
  .dialog {
    width: min(400px, calc(100vw - 48px));
    padding: var(--space-6);
    border: 0;
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
  .title {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
    letter-spacing: var(--tracking-15);
    overflow-wrap: anywhere;
  }
  .body {
    margin: var(--space-2) 0 0;
    color: var(--color-text-muted);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-6);
  }
</style>
