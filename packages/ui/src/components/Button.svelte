<!--
  A text button. One primary per view (DESIGN-PRINCIPLES.md, Restraint):
  solid accent (ink on light, white on dark), never a gradient. Hover is an instant overlay that fades
  out in 100 ms; press is one more overlay step, never a scale (APP-DESIGN #24).
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  interface Props extends HTMLButtonAttributes {
    variant?: "primary" | "secondary" | "ghost" | "danger";
    size?: "md" | "lg";
    icon?: Snippet;
    children: Snippet;
    /** The button element, for focusing it. */
    ref?: HTMLButtonElement;
  }

  let {
    variant = "secondary",
    size = "md",
    icon,
    children,
    type = "button",
    ref = $bindable(),
    ...rest
  }: Props = $props();
</script>

<button bind:this={ref} class="button {variant} {size}" {type} {...rest}>
  {#if icon}<span class="icon" aria-hidden="true">{@render icon()}</span>{/if}
  <span class="label">{@render children()}</span>
</button>

<style>
  .button {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    height: var(--control-height);
    padding: 0 var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    font-size: var(--text-13);
    line-height: var(--leading-13);
    font-weight: var(--weight-medium);
    white-space: nowrap;
    isolation: isolate;
  }
  .lg {
    height: var(--control-height-lg);
    padding: 0 var(--space-4);
  }
  .icon {
    display: inline-flex;
    margin-left: calc(-1 * var(--space-1));
  }

  /* The state overlay: instant in, 100 ms out. */
  .button::after {
    content: "";
    position: absolute;
    inset: -1px;
    border-radius: inherit;
    background: var(--color-overlay-hover);
    opacity: 0;
    transition: opacity var(--duration-hover-out) linear;
    pointer-events: none;
  }
  .button:hover:not(:disabled)::after {
    opacity: 1;
    transition: none;
  }
  .button:active:not(:disabled)::after {
    background: var(--color-overlay-press);
    opacity: 1;
    transition: none;
  }

  .primary {
    background: var(--color-accent);
    color: var(--color-on-accent);
  }
  /* The accent has its own hover and press steps, not the overlay. */
  .primary:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }
  .primary:active:not(:disabled) {
    background: var(--color-accent-press);
  }
  .primary::after {
    display: none;
  }

  .danger {
    background: var(--color-danger);
    color: var(--color-on-danger);
  }
  .danger:hover:not(:disabled) {
    background: var(--color-danger-hover);
  }
  .danger::after {
    display: none;
  }

  .secondary {
    background: var(--color-raised);
    border-color: var(--color-hairline-strong);
    color: var(--color-text);
  }

  .ghost {
    background: transparent;
    color: var(--color-text);
  }

  .button:disabled {
    opacity: 0.45;
  }

  @media (forced-colors: active) {
    .button {
      border-color: ButtonText;
    }
  }
</style>
