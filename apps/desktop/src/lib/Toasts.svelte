<!--
  Quiet in-window notices at the bottom of the list: "Removed … Undo", a
  copied link, an action that failed. They float, so they get the floating
  elevation (APP-DESIGN #22); they never take focus and they leave on their
  own. Screen readers hear them through a polite live region.
-->
<script lang="ts">
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { app } from "./app.svelte";

  const reduced =
    typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each app.toasts as toast (toast.id)}
    <div
      class="toast"
      in:fly={{ y: reduced ? 0 : 8, duration: reduced ? 0 : 200, easing: cubicOut }}
      out:fly={{ y: 0, duration: reduced ? 0 : 130 }}
    >
      <span class="text">{toast.text}</span>
      {#if toast.action}
        <button type="button" class="action" onclick={toast.action.run}>{toast.action.label}</button
        >
      {/if}
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: absolute;
    left: 50%;
    bottom: var(--space-5);
    z-index: 10;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    transform: translateX(-50%);
    pointer-events: none;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    min-height: 36px;
    max-width: min(520px, 80vw);
    padding: 0 var(--space-2) 0 var(--space-4);
    border-radius: var(--radius-floating);
    background: var(--color-raised);
    box-shadow: var(--elevation-floating);
    pointer-events: auto;
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding-right: var(--space-2);
  }
  .action {
    position: relative;
    height: var(--control-height);
    padding: 0 var(--space-3);
    border: 0;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-accent);
    font-weight: var(--weight-medium);
  }
  .action:hover {
    background: var(--color-overlay-hover);
  }
  .action:active {
    background: var(--color-overlay-press);
  }
</style>
