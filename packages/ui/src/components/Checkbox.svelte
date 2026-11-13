<!--
  A checkbox in the Windows 11 manner: a 16 px box with a 4 px radius, solid
  accent (ink on light, white on dark) with a contrasting check when on, a dash when mixed. The native input
  stays underneath (keyboard, forms, screen readers); only its look changes.
  The label is the children, or `label` when nothing visible is wanted.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLInputAttributes } from "svelte/elements";

  interface Props extends Omit<HTMLInputAttributes, "type"> {
    checked?: boolean;
    indeterminate?: boolean;
    /** The accessible name when there are no children. */
    label?: string;
    children?: Snippet;
  }

  let {
    checked = $bindable(false),
    indeterminate = false,
    label,
    children,
    ...rest
  }: Props = $props();
</script>

<label class="checkbox">
  <input type="checkbox" bind:checked {indeterminate} aria-label={label} {...rest} />
  <span class="box" aria-hidden="true">
    <svg viewBox="0 0 16 16" width="16" height="16">
      {#if indeterminate}
        <path d="M4.5 8h7" />
      {:else}
        <path d="M4 8.25l2.75 2.75L12 5.5" />
      {/if}
    </svg>
  </span>
  {#if children}<span class="text">{@render children()}</span>{/if}
</label>

<style>
  .checkbox {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 24px;
    cursor: default;
  }
  input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: default;
  }
  .box {
    position: relative;
    display: inline-grid;
    place-items: center;
    flex: none;
    width: 16px;
    height: 16px;
    border: 1px solid var(--color-control-stroke);
    border-radius: var(--radius-control);
    background: var(--color-field);
    color: var(--color-on-accent);
  }
  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: square;
    opacity: 0;
  }
  input:checked + .box,
  input:indeterminate + .box {
    border-color: var(--color-accent);
    background: var(--color-accent);
  }
  input:checked + .box svg,
  input:indeterminate + .box svg {
    opacity: 1;
  }
  .checkbox:hover input:not(:disabled) + .box {
    border-color: var(--color-text-muted);
  }
  .checkbox:hover input:checked:not(:disabled) + .box {
    border-color: var(--color-accent-hover);
    background: var(--color-accent-hover);
  }
  input:focus-visible + .box {
    outline: var(--focus-ring);
    outline-offset: var(--focus-offset);
  }
  input:disabled + .box {
    opacity: 0.45;
  }

  @media (forced-colors: active) {
    .box {
      border-color: ButtonText;
    }
    input:checked + .box {
      background: Highlight;
      color: HighlightText;
    }
  }
</style>
