<!--
  A keyboard shortcut drawn as keycaps: "Ctrl+K" becomes [Ctrl] [K]. The
  shortcut text comes from the string catalog; `+` separates the keys.
-->
<script lang="ts">
  interface Props {
    shortcut: string;
    size?: "sm" | "md";
  }

  let { shortcut, size = "sm" }: Props = $props();
  const keys = $derived(shortcut.split("+").map((key) => key.trim()));
</script>

<kbd class="keycaps {size}">
  <span class="visually-hidden">{shortcut}</span>
  {#each keys as key, i (i)}<kbd class="key" aria-hidden="true">{key}</kbd>{/each}
</kbd>

<style>
  .keycaps {
    display: inline-flex;
    gap: 2px;
    font: inherit;
  }
  .key {
    display: inline-grid;
    place-items: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border: 1px solid var(--color-hairline-strong);
    border-bottom-width: 2px;
    border-radius: var(--radius-control);
    background: var(--color-raised);
    color: var(--color-text-muted);
    font-family: var(--font-sans);
    font-size: 11px;
    line-height: 1;
    font-weight: var(--weight-medium);
  }
  .md .key {
    min-width: 24px;
    height: 24px;
    padding: 0 7px;
    font-size: var(--text-12);
  }
</style>
