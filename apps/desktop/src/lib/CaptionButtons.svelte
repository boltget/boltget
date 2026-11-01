<!--
  Minimize, maximize and close, drawn like Windows 11's own: 46 px wide, the
  full title bar height (so the top edge stays a target when maximized),
  Segoe Fluent Icons glyphs, red close hover. On Windows a native window lies
  over the maximize button for snap layouts and reports its hover and press
  (src-tauri/src/platform/windows.rs); the page only draws them.
-->
<script lang="ts">
  import { t } from "@boltget/ui/messages";
  import { onMount } from "svelte";
  import {
    GLYPHS,
    captionRect,
    detectGlyphSource,
    sameRect,
    type CaptionRect,
    type GlyphSource,
  } from "./caption";
  import {
    appWindow,
    onMaximizeButton,
    setCaptionLayout,
    type MaximizeButtonState,
  } from "./native";

  let maximized = $state(false);
  let nativeState = $state<MaximizeButtonState>("rest");
  let glyphs = $state<GlyphSource>("font");
  let maximizeButton: HTMLButtonElement | undefined = $state();

  const maximizeLabel = $derived(maximized ? t("caption.restore") : t("caption.maximize"));

  onMount(() => {
    glyphs = detectGlyphSource();
    const cleanups: Array<() => void> = [];
    let last: CaptionRect | undefined;

    const report = () => {
      if (!maximizeButton) return;
      const rect = captionRect(maximizeButton.getBoundingClientRect(), innerWidth);
      if (sameRect(last, rect)) return;
      last = rect;
      void setCaptionLayout(rect);
    };
    report();
    const observer = new ResizeObserver(report);
    if (maximizeButton) observer.observe(maximizeButton);
    cleanups.push(() => observer.disconnect());

    void appWindow
      .onMaximizedChange((value) => (maximized = value))
      .then((off) => cleanups.push(off));
    void onMaximizeButton((state) => (nativeState = state)).then((off) => cleanups.push(off));

    return () => cleanups.forEach((cleanup) => cleanup());
  });
</script>

<div class="captions">
  <button
    class="caption"
    type="button"
    aria-label={t("caption.minimize")}
    title={t("caption.minimize")}
    tabindex="-1"
    onclick={() => appWindow.minimize()}
  >
    {#if glyphs === "font"}
      <span class="glyph" aria-hidden="true">{GLYPHS.minimize}</span>
    {:else}
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5.5h10" /></svg>
    {/if}
  </button>
  <button
    bind:this={maximizeButton}
    class="caption"
    data-native={nativeState}
    type="button"
    aria-label={maximizeLabel}
    title={maximizeLabel}
    tabindex="-1"
    onclick={() => appWindow.toggleMaximize()}
  >
    {#if glyphs === "font"}
      <span class="glyph" aria-hidden="true">{maximized ? GLYPHS.restore : GLYPHS.maximize}</span>
    {:else if maximized}
      <svg viewBox="0 0 10 10" aria-hidden="true"
        ><path d="M2.5 2.5V.5h7v7h-2M.5 2.5h7v7h-7Z" /></svg
      >
    {:else}
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M.5.5h9v9h-9Z" /></svg>
    {/if}
  </button>
  <button
    class="caption close"
    type="button"
    aria-label={t("caption.close")}
    title={t("caption.close")}
    tabindex="-1"
    onclick={() => appWindow.close()}
  >
    {#if glyphs === "font"}
      <span class="glyph" aria-hidden="true">{GLYPHS.close}</span>
    {:else}
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M.5.5l9 9M9.5.5l-9 9" /></svg>
    {/if}
  </button>
</div>

<style>
  .captions {
    display: flex;
    align-self: stretch;
  }
  .caption {
    display: grid;
    place-items: center;
    width: var(--caption-button-width);
    height: 100%;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--color-text);
    transition: background-color var(--duration-hover-out) linear;
  }
  .glyph {
    font-family: var(--font-caption);
    font-size: 10px;
    line-height: 1;
  }
  svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
  }

  /* Hover in and press are instant, hover out fades in 100 ms (APP-DESIGN #24). */
  .caption:hover,
  .caption[data-native="hover"] {
    background: var(--color-overlay-hover);
    transition: none;
  }
  .caption:active,
  .caption[data-native="press"] {
    background: var(--color-overlay-press);
    transition: none;
  }
  .close:hover {
    background: var(--color-caption-close);
    color: var(--color-on-caption-close);
  }
  .close:active {
    background: var(--color-caption-close-press);
    color: var(--color-on-caption-close);
  }

  /* An inactive window's captions are dimmed, as Windows draws them. */
  :global(.titlebar.inactive) .caption:not(:hover) {
    color: var(--color-text-subtle);
  }

  @media (forced-colors: active) {
    .caption:hover,
    .caption[data-native="hover"] {
      background: Highlight;
      color: HighlightText;
    }
  }
</style>
