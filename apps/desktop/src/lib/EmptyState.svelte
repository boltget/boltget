<!--
  The empty state is the add flow (APP-DESIGN #5): one sentence, the Ctrl+V
  keycap and the pixel mark, played once when the window appears (one of the
  two places the signature animation is allowed, #28).
-->
<script lang="ts">
  import Keycap from "@boltget/ui/components/Keycap.svelte";
  import Mark from "@boltget/ui/components/Mark.svelte";
  import { t } from "@boltget/ui/messages";
  import { onMount } from "svelte";
  import { windowShown } from "./lifecycle";

  let waiting = $state(true);

  onMount(() => {
    void windowShown.then(() => (waiting = false));
  });
</script>

<section class="empty">
  <Mark size={108} arrive paused={waiting} />
  <h1 class="title">{t("empty.title")}</h1>
  <p class="hint">
    <span class="visually-hidden">{t("empty.shortcut", { shortcut: t("shortcut.paste") })}</span>
    <span aria-hidden="true"><Keycap shortcut={t("shortcut.paste")} size="md" /></span>
  </p>
</section>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-4);
    height: 100%;
    padding: var(--space-8);
    /* Optically centred: a little above the middle. */
    padding-bottom: calc(var(--space-12) * 2);
    text-align: center;
  }
  .title {
    margin: var(--space-4) 0 0;
    font-size: var(--text-20);
    line-height: var(--leading-20);
    font-weight: var(--weight-medium);
    letter-spacing: var(--tracking-20);
  }
  .hint {
    margin: 0;
  }
</style>
