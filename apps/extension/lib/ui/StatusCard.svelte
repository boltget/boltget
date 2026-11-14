<!--
  The connection status: a dot, a title and a hint, and the download page when
  the app is missing or too old.
-->
<script lang="ts">
  import type { ConnectionStatus } from "../native";
  import { t } from "../i18n";
  import { DOWNLOAD_URL } from "./connection";
  import { statusText } from "./status";

  interface Props {
    status: ConnectionStatus | null;
  }

  let { status }: Props = $props();
  const text = $derived(statusText(status));
</script>

<div class="status" data-state={status?.state ?? "checking"} role="status">
  <span class="dot {text.tone}" aria-hidden="true"></span>
  <div class="words">
    <p class="title">{text.title}</p>
    {#if text.hint}<p class="hint">{text.hint}</p>{/if}
    {#if text.getApp}
      <a class="get" href={DOWNLOAD_URL} target="_blank" rel="noreferrer">{t("status.get_app")}</a>
    {/if}
  </div>
</div>

<style>
  .status {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
  }
  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    margin-top: 6px;
    border-radius: 50%;
    background: var(--color-progress-idle);
  }
  .dot.ok {
    background: var(--color-success);
  }
  .dot.problem {
    background: var(--color-error);
  }
  .words {
    min-width: 0;
  }
  p {
    margin: 0;
  }
  .title {
    font-weight: var(--weight-medium);
  }
  .hint {
    margin-top: 2px;
    color: var(--color-text-muted);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .get {
    display: inline-block;
    margin-top: var(--space-2);
    color: var(--color-text);
    font-weight: var(--weight-medium);
    text-underline-offset: 3px;
  }
  .get:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-offset);
    border-radius: 2px;
  }
</style>
