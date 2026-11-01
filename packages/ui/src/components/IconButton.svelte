<!--
  A square button with an icon only. `label` is required: it is the accessible
  name and the tooltip, so an icon is never the only explanation.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  interface Props extends HTMLButtonAttributes {
    label: string;
    size?: "md" | "lg";
    pressed?: boolean;
    children: Snippet;
  }

  let { label, size = "md", pressed, children, type = "button", ...rest }: Props = $props();
</script>

<button
  class="icon-button {size}"
  {type}
  aria-label={label}
  title={label}
  aria-pressed={pressed}
  {...rest}
>
  <span aria-hidden="true">{@render children()}</span>
</button>

<style>
  .icon-button {
    position: relative;
    display: inline-grid;
    place-items: center;
    width: var(--control-height);
    height: var(--control-height);
    padding: 0;
    border: 0;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-muted);
  }
  .lg {
    width: var(--control-height-lg);
    height: var(--control-height-lg);
  }
  .icon-button::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: var(--color-overlay-hover);
    opacity: 0;
    transition: opacity var(--duration-hover-out) linear;
    pointer-events: none;
  }
  .icon-button:hover:not(:disabled) {
    color: var(--color-text);
  }
  .icon-button:hover:not(:disabled)::after {
    opacity: 1;
    transition: none;
  }
  .icon-button:active:not(:disabled)::after,
  .icon-button[aria-pressed="true"]::after {
    background: var(--color-overlay-press);
    opacity: 1;
    transition: none;
  }
  .icon-button:disabled {
    opacity: 0.45;
  }
</style>
