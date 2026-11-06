<!--
  One setting in the Windows 11 manner (APP-DESIGN #17): a label, a
  one-sentence description and the control on the right; anything more
  (a category's fields, a proxy's address) below it, inside the same card.
  A problem with the value shows under it in words, tied to the control
  through `problemId` (the control sets aria-describedby to it).
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { RowId } from "./registry";

  interface Props {
    id: RowId | string;
    label: string;
    description?: string;
    /** Highlighted once, when the palette or a search jumps here. */
    target?: boolean;
    problem?: string | null;
    control?: Snippet;
    children?: Snippet;
  }

  let { id, label, description, target = false, problem, control, children }: Props = $props();
</script>

<section class="row" class:target data-setting={id} aria-labelledby="setting-{id}">
  <div class="head">
    <div class="text">
      <h3 class="label" id="setting-{id}">{label}</h3>
      {#if description}<p class="description" id="setting-{id}-description">{description}</p>{/if}
    </div>
    {#if control}<div class="control">{@render control()}</div>{/if}
  </div>
  {#if children}<div class="body">{@render children()}</div>{/if}
  {#if problem}
    <p class="problem" id="setting-{id}-problem" role="alert">{problem}</p>
  {/if}
</section>

<style>
  .row {
    padding: var(--space-4);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-raised);
    scroll-margin: var(--space-6);
  }
  .row.target {
    animation: arrive 1.2s var(--ease-exit);
  }
  @keyframes arrive {
    from {
      box-shadow: 0 0 0 2px var(--color-accent-tint);
      border-color: var(--color-accent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .row.target {
      animation: none;
      border-color: var(--color-accent);
    }
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-6);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .label {
    margin: 0;
    font-size: var(--text-13);
    line-height: var(--leading-13);
    font-weight: var(--weight-medium);
  }
  .description {
    margin: 2px 0 0;
    color: var(--color-text-muted);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-secondary);
  }
  .control {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: none;
  }
  .body {
    margin-top: var(--space-3);
  }
  .problem {
    margin: var(--space-2) 0 0;
    color: var(--color-error);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-secondary);
  }
</style>
