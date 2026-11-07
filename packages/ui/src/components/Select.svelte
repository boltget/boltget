<!--
  A drop-down choice (the Windows 11 combo box): the native <select>, so the
  keyboard, the screen reader and the list itself behave as the system
  does; only the closed control is drawn to match the other controls.
-->
<script lang="ts" generics="T extends string">
  import type { HTMLSelectAttributes } from "svelte/elements";

  interface Props extends Omit<HTMLSelectAttributes, "value"> {
    value: T;
    options: readonly { value: T; label: string }[];
  }

  let { value = $bindable(), options, ...rest }: Props = $props();
</script>

<span class="select">
  <select bind:value {...rest}>
    {#each options as option (option.value)}
      <option value={option.value}>{option.label}</option>
    {/each}
  </select>
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <path d="M3.5 6 8 10.5 12.5 6" />
  </svg>
</span>

<style>
  .select {
    position: relative;
    display: inline-flex;
    align-items: center;
    min-width: 200px;
  }
  select {
    width: 100%;
    height: var(--control-height);
    padding: 0 calc(var(--space-2) + 22px) 0 var(--space-3);
    border: 1px solid var(--color-hairline-strong);
    border-bottom-color: var(--color-control-stroke);
    border-radius: var(--radius-control);
    background: var(--color-raised);
    color: var(--color-text);
    font: inherit;
    appearance: none;
    outline: none;
  }
  select:hover:not(:disabled) {
    background:
      linear-gradient(var(--color-overlay-hover), var(--color-overlay-hover)), var(--color-raised);
  }
  select:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-offset);
  }
  option {
    background: var(--color-raised);
    color: var(--color-text);
  }
  svg {
    position: absolute;
    right: var(--space-2);
    fill: none;
    stroke: var(--color-text-muted);
    stroke-width: 1.25;
    pointer-events: none;
  }

  @media (forced-colors: active) {
    select {
      border-color: ButtonText;
    }
    svg {
      stroke: ButtonText;
    }
  }
</style>
