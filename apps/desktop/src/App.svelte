<!--
  The window: title bar, sidebar and the content area (APP-DESIGN #1). The
  chrome is one grey surface; the content sits on it as a white sheet with a
  hairline edge, like the website's hero card.
-->
<script lang="ts">
  import EmptyState from "./lib/EmptyState.svelte";
  import Sidebar from "./lib/Sidebar.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
</script>

<div class="window">
  <TitleBar />
  <div class="body">
    <Sidebar />
    <main class="content">
      <EmptyState />
    </main>
  </div>
</div>

<style>
  .window {
    display: grid;
    grid-template-rows: var(--titlebar-height) minmax(0, 1fr);
    height: 100vh;
    background: var(--color-chrome);
  }
  .body {
    display: flex;
    min-height: 0;
  }
  .content {
    flex: 1;
    min-width: 0;
    overflow: auto;
    background: var(--color-content);
    border-top: 1px solid var(--color-hairline);
    border-left: 1px solid var(--color-hairline);
    border-top-left-radius: var(--radius-floating);
  }
</style>
