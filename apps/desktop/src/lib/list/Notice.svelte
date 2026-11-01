<!--
  A calm message in place of the list: the engine starting, locked by the
  CLI or failed, or a view with nothing in it. No spinners and no
  illustrations (APP-DESIGN #28): an icon, one line, at most one more.
-->
<script lang="ts">
  import Icon, { type IconName } from "../Icon.svelte";

  interface Props {
    icon?: IconName;
    title: string;
    body?: string;
    detail?: string;
    tone?: "neutral" | "warning" | "error";
    /** Fade in after a moment, for states that are usually brief. */
    delayed?: boolean;
  }

  let { icon, title, body, detail, tone = "neutral", delayed = false }: Props = $props();
</script>

<div class="notice {tone}" class:delayed role="status">
  {#if icon}<span class="icon"><Icon name={icon} size={20} /></span>{/if}
  <p class="title">{title}</p>
  {#if body}<p class="body">{body}</p>{/if}
  {#if detail}<p class="detail">{detail}</p>{/if}
</div>

<style>
  .notice {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    height: 100%;
    padding: var(--space-8) var(--space-8) calc(var(--space-12) * 2);
    text-align: center;
  }
  .delayed {
    animation: appear var(--duration-pane) var(--ease-enter) 400ms both;
  }
  @keyframes appear {
    from {
      opacity: 0;
    }
  }
  .icon {
    display: inline-flex;
    margin-bottom: var(--space-2);
    color: var(--color-text-subtle);
  }
  .warning .icon {
    color: var(--color-warning);
  }
  .error .icon {
    color: var(--color-error);
  }
  p {
    margin: 0;
    max-width: 420px;
  }
  .title {
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-medium);
  }
  .body {
    color: var(--color-text-muted);
  }
  .detail {
    margin-top: var(--space-1);
    color: var(--color-text-subtle);
    font-family: var(--font-mono);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    user-select: text;
    -webkit-user-select: text;
    cursor: text;
    overflow-wrap: anywhere;
  }
  @media (prefers-reduced-motion: reduce) {
    .delayed {
      animation: none;
    }
  }
</style>
