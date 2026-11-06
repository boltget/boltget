<!--
  An on/off switch in the Windows 11 manner: a 40 × 20 px track with a knob
  that slides across, solid emerald when on. The native checkbox stays
  underneath with `role="switch"` (keyboard, screen readers); only its look
  changes. Settings apply when it flips, so there is no Save.
-->
<script lang="ts">
  import type { HTMLInputAttributes } from "svelte/elements";

  interface Props extends Omit<HTMLInputAttributes, "type" | "role"> {
    checked?: boolean;
    /** The accessible name (the row's label is usually tied with aria-labelledby). */
    label?: string;
  }

  let { checked = $bindable(false), label, ...rest }: Props = $props();
</script>

<span class="switch">
  <input type="checkbox" role="switch" bind:checked aria-label={label} {...rest} />
  <span class="track" aria-hidden="true"><span class="knob"></span></span>
</span>

<style>
  .switch {
    position: relative;
    display: inline-flex;
    flex: none;
    width: 40px;
    height: 20px;
  }
  input {
    position: absolute;
    inset: -2px;
    width: calc(100% + 4px);
    height: calc(100% + 4px);
    margin: 0;
    opacity: 0;
    cursor: default;
  }
  .track {
    display: inline-flex;
    align-items: center;
    width: 100%;
    height: 100%;
    padding: 0 4px;
    border: 1px solid var(--color-text-muted);
    border-radius: 10px;
    background: var(--color-field);
    transition: background-color var(--duration-small) var(--ease-enter);
  }
  .knob {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--color-text-muted);
    transition: transform var(--duration-small) var(--ease-enter);
  }
  input:checked + .track {
    border-color: var(--color-accent);
    background: var(--color-accent);
  }
  input:checked + .track .knob {
    background: var(--color-on-accent);
    transform: translateX(18px);
  }
  .switch:hover input:not(:disabled) + .track {
    background: var(--color-overlay-hover);
  }
  .switch:hover input:checked:not(:disabled) + .track {
    border-color: var(--color-accent-hover);
    background: var(--color-accent-hover);
  }
  input:focus-visible + .track {
    outline: var(--focus-ring);
    outline-offset: var(--focus-offset);
  }
  input:disabled + .track {
    opacity: 0.45;
  }

  @media (forced-colors: active) {
    .track {
      border-color: ButtonText;
    }
    .knob {
      background: ButtonText;
    }
    input:checked + .track {
      background: Highlight;
    }
    input:checked + .track .knob {
      background: HighlightText;
    }
  }
</style>
