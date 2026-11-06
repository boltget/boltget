<!--
  A one-line text field: 28 px high, a hairline that turns emerald on focus
  and red when the value has a problem (`invalid`; the problem itself is
  text under the field, tied with aria-describedby). `unit` is a muted word
  after the field, such as "MiB/s"; `mono` is for paths and addresses.
-->
<script lang="ts">
  import type { HTMLInputAttributes } from "svelte/elements";

  interface Props extends HTMLInputAttributes {
    value?: string;
    invalid?: boolean;
    unit?: string;
    mono?: boolean;
    /** The input element, for focusing it. */
    ref?: HTMLInputElement;
  }

  let {
    value = $bindable(""),
    invalid = false,
    unit,
    mono = false,
    ref = $bindable(),
    type = "text",
    ...rest
  }: Props = $props();
</script>

<span class="text-field" class:mono>
  <input
    bind:this={ref}
    bind:value
    {type}
    aria-invalid={invalid || undefined}
    spellcheck="false"
    autocomplete="off"
    {...rest}
  />
  {#if unit}<span class="unit">{unit}</span>{/if}
</span>

<style>
  .text-field {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  input {
    flex: 1;
    width: 100%;
    min-width: 0;
    height: var(--control-height);
    padding: 0 var(--space-2);
    border: 1px solid var(--color-hairline-strong);
    border-radius: var(--radius-control);
    background: var(--color-field);
    color: var(--color-text);
    font: inherit;
    outline: none;
  }
  input::placeholder {
    color: var(--color-text-subtle);
  }
  input:focus {
    border-color: var(--color-focus);
    box-shadow: 0 0 0 1px var(--color-focus);
  }
  input[aria-invalid="true"] {
    border-color: var(--color-error);
  }
  input[aria-invalid="true"]:focus {
    box-shadow: 0 0 0 1px var(--color-error);
  }
  .mono input {
    font-family: var(--font-mono);
    font-size: var(--text-12);
  }
  .mono input::placeholder {
    font-family: var(--font-sans);
    font-size: var(--text-13);
  }
  .unit {
    flex: none;
    color: var(--color-text-subtle);
  }

  @media (forced-colors: active) {
    input {
      border-color: ButtonText;
    }
  }
</style>
