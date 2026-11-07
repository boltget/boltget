<!--
  A finished queue is about to put the computer to sleep or shut it down
  (APP-DESIGN, "Queues as built"). The backend counts 60 seconds and brings
  the window forward; this shows the time left with Cancel, which has focus
  so Enter or Esc cancels. The only dialog the app opens on its own: an
  action that turns the computer off must never happen unseen.
-->
<script lang="ts">
  import { trapTab } from "../focus";
  import Button from "@boltget/ui/components/Button.svelte";
  import { t } from "@boltget/ui/messages";
  import { app } from "../app.svelte";
  import { downloads } from "../engine";

  let dialog: HTMLDialogElement | undefined = $state();
  let cancel: HTMLButtonElement | undefined = $state();
  let now = $state(Date.now());

  const countdown = $derived(downloads.countdown);
  const left = $derived(countdown ? Math.max(0, Math.ceil((countdown.deadline - now) / 1000)) : 0);
  const time = $derived(`${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`);

  $effect(() => {
    if (!dialog) return;
    if (countdown && !dialog.open) {
      now = Date.now();
      dialog.showModal();
      cancel?.focus();
    } else if (!countdown && dialog.open) dialog.close();
  });

  $effect(() => {
    if (!countdown) return;
    const timer = setInterval(() => (now = Date.now()), 250);
    return () => clearInterval(timer);
  });

  function stop() {
    if (!downloads.countdown) return;
    void downloads.cancelFinish();
    app.toast(t("countdown.cancelled"));
  }
</script>

<dialog
  bind:this={dialog}
  use:trapTab
  class="dialog"
  role="alertdialog"
  aria-labelledby="countdown-title"
  aria-describedby="countdown-body"
  oncancel={(event) => {
    event.preventDefault();
    stop();
  }}
>
  {#if countdown}
    <h2 id="countdown-title" class="title">
      {countdown.action === "sleep"
        ? t("countdown.title.sleep", { time })
        : t("countdown.title.shut_down", { time })}
    </h2>
    <p id="countdown-body" class="body">
      {t("countdown.body", { queue: app.queueName(countdown.queue) })}
    </p>
    <div class="bar" aria-hidden="true">
      <span class="fill" style:transform="scaleX({left / 60})"></span>
    </div>
    <div class="buttons">
      <Button bind:ref={cancel} variant="primary" onclick={stop}>{t("countdown.cancel")}</Button>
    </div>
  {/if}
</dialog>

<style>
  .dialog {
    width: min(420px, calc(100vw - 48px));
    padding: var(--space-6);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-raised);
    color: var(--color-text);
    box-shadow: var(--elevation-floating);
  }
  .dialog::backdrop {
    background: var(--color-scrim);
  }
  .title {
    margin: 0;
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
    font-variant-numeric: tabular-nums;
  }
  .body {
    margin: var(--space-2) 0 var(--space-4);
    color: var(--color-text-muted);
  }
  /* A chunk: square, like every progress bar (#26). */
  .bar {
    position: relative;
    height: 4px;
    overflow: hidden;
    background: var(--color-progress-track);
  }
  .fill {
    position: absolute;
    inset: 0;
    transform-origin: left center;
    background: var(--color-warning);
    transition: transform var(--duration-tick) linear;
  }
  @media (prefers-reduced-motion: reduce) {
    .fill {
      transition: none;
    }
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--space-6);
  }
  @media (forced-colors: active) {
    .bar {
      outline: 1px solid CanvasText;
    }
    .fill {
      forced-color-adjust: none;
      background: Highlight;
    }
  }
</style>
