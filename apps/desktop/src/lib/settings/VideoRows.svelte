<!--
  Settings, Video (docs/APP-DESIGN.md, "Video as built"; docs/VIDEO.md):
  the video tools Boltget installs (their versions, the last update check,
  Check for updates now, Install, Reinstall and Remove), yt-dlp's update
  channel, the container the picker prefers, and programs of the user's
  own instead of the installed tools. Rows apply as they change, like the
  rest of Settings; a path applies on Enter or when its field is left,
  once the program has started and reported its version.
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import IconButton from "@boltget/ui/components/IconButton.svelte";
  import Select from "@boltget/ui/components/Select.svelte";
  import Switch from "@boltget/ui/components/Switch.svelte";
  import TextField from "@boltget/ui/components/TextField.svelte";
  import { t } from "@boltget/ui/messages";
  import { onMount, untrack } from "svelte";
  import { app } from "../app.svelte";
  import type {
    MediaChannel,
    MediaInstallEvent,
    MediaTool,
    MediaToolsInfo,
    SettingsPatch,
    VideoSettings,
  } from "../bindings";
  import { clock } from "../clock.svelte";
  import { downloads, errorText } from "../engine";
  import { formatWhen } from "../format";
  import Icon from "../Icon.svelte";
  import { installLines, toolName } from "../media/picker";
  import type { RowId } from "./registry";
  import SettingRow from "./SettingRow.svelte";

  interface Props {
    video: VideoSettings;
    shown: Set<RowId>;
    highlighted: RowId | null;
    problems: Record<string, string | null>;
    problem: (field: string, text: string | null) => void;
    save: (field: string, patch: SettingsPatch) => Promise<boolean>;
  }

  let { video, shown, highlighted, problems, problem, save }: Props = $props();

  let info = $state.raw<MediaToolsInfo | null>(null);
  let busy = $state<"check" | "install" | "remove" | null>(null);
  let events = $state.raw<MediaInstallEvent[]>([]);
  let editing = $state<MediaTool | null>(null);
  let paths = $state<Record<MediaTool, string>>({ ytDlp: "", ffmpeg: "" });

  const installed = $derived(info?.tools.some((x) => x.version !== null) ?? false);
  const missing = $derived(
    info?.tools.some((x) => x.version === null && x.customPath === null) ?? true,
  );
  const lastCheck = $derived(
    info?.tools.reduce<number | null>(
      (latest, x) => (x.lastCheck !== null && x.lastCheck > (latest ?? 0) ? x.lastCheck : latest),
      null,
    ) ?? null,
  );
  const progress = $derived(installLines([], events));

  onMount(() => void refresh());

  // Drafts follow the settings, except the field being typed in.
  $effect(() => {
    const current = video;
    untrack(() => {
      if (editing !== "ytDlp" && !problems.path_ytDlp) paths.ytDlp = current.ytDlpPath ?? "";
      if (editing !== "ffmpeg" && !problems.path_ffmpeg) paths.ffmpeg = current.ffmpegPath ?? "";
    });
  });

  async function refresh() {
    const result = await downloads.mediaTools();
    if (result.status === "ok") info = result.data;
    else problem("video_tools", errorText(result.error));
  }

  function toolLine(x: MediaToolsInfo["tools"][number]): string {
    const name = toolName(x.tool);
    if (x.customPath) {
      return x.customVersion
        ? t("settings.video.tool.custom", { name, version: x.customVersion })
        : t("settings.video.tool.custom_broken", { name });
    }
    return x.version
      ? t("settings.video.tool.installed", { name, version: x.version })
      : t("settings.video.tool.missing", { name });
  }

  async function check() {
    busy = "check";
    problem("video_tools", null);
    const result = await downloads.updateMediaTools();
    busy = null;
    if (result.status === "error") return problem("video_tools", errorText(result.error));
    info = result.data;
    app.toast(t("settings.video.up_to_date"));
  }

  async function install() {
    busy = "install";
    events = [];
    problem("video_tools", null);
    const result = await downloads.installMedia((e) => (events = [...events, e]));
    busy = null;
    events = [];
    if (result.status === "error") return problem("video_tools", errorText(result.error));
    info = result.data;
    app.toast(t("settings.video.installed"));
  }

  async function remove(): Promise<boolean> {
    busy = "remove";
    problem("video_tools", null);
    const result = await downloads.removeMediaTools();
    busy = null;
    if (result.status === "error") {
      problem("video_tools", errorText(result.error));
      return false;
    }
    info = result.data;
    return true;
  }

  function confirmRemove() {
    app.confirmation = {
      title: t("settings.video.remove.title"),
      body: t("settings.video.remove.body"),
      confirm: t("settings.video.remove"),
      run: () =>
        void remove().then((done) => {
          if (done) app.toast(t("settings.video.removed"));
        }),
    };
  }

  async function reinstall() {
    if (await remove()) await install();
  }

  async function setChannel(channel: MediaChannel) {
    const result = await downloads.setMediaChannel(channel);
    if (result.status === "error") return problem("video_channel", errorText(result.error));
    problem("video_channel", null);
    info = result.data;
  }

  async function commitPath(tool: MediaTool) {
    editing = null;
    const field = `path_${tool}`;
    const typed = paths[tool].trim();
    const current = (tool === "ytDlp" ? video.ytDlpPath : video.ffmpegPath) ?? "";
    if (typed === current) return problem(field, null);
    const result = await downloads.setMediaToolPath(tool, typed || null);
    if (result.status === "error") return problem(field, errorText(result.error));
    problem(field, null);
    info = result.data;
  }

  function onPathKey(event: KeyboardEvent, tool: MediaTool) {
    if (event.key === "Enter") {
      event.preventDefault();
      void commitPath(tool);
    } else if (event.key === "Escape") {
      event.stopPropagation();
      editing = null;
      problem(`path_${tool}`, null);
      paths[tool] = (tool === "ytDlp" ? video.ytDlpPath : video.ffmpegPath) ?? "";
      (event.currentTarget as HTMLElement).blur();
    }
  }

  function clearPath(tool: MediaTool) {
    paths[tool] = "";
    void commitPath(tool);
  }
</script>

{#if shown.has("video_tools")}
  <SettingRow
    id="video_tools"
    label={t("settings.video.tools")}
    description={t("settings.video.tools.description")}
    target={highlighted === "video_tools"}
    problem={problems.video_tools}
  >
    <div class="tools" aria-live="polite">
      {#if busy === "install"}
        <ul class="lines">
          {#each progress as line (line.tool)}
            <li>
              <span class="name">{line.name}</span>
              <span class="note">{line.text}</span>
            </li>
          {/each}
        </ul>
      {:else if info}
        <ul class="lines">
          {#each info.tools as x (x.tool)}
            <li>
              <Icon name={x.version || x.customVersion ? "check" : "info"} size={14} />
              <span>{toolLine(x)}</span>
            </li>
          {/each}
        </ul>
        {#if installed}
          <p class="note">
            {lastCheck
              ? t("settings.video.checked", { when: formatWhen(lastCheck, clock.now) })
              : t("settings.video.never_checked")}
          </p>
        {/if}
      {/if}
      <div class="buttons">
        {#if installed}
          <Button disabled={busy !== null} onclick={() => void check()}>
            {busy === "check" ? t("settings.video.checking") : t("settings.video.check")}
          </Button>
        {/if}
        {#if missing}
          <Button variant="primary" disabled={busy !== null} onclick={() => void install()}>
            {t("settings.video.install")}
          </Button>
        {/if}
        {#if installed}
          <Button disabled={busy !== null} onclick={() => void reinstall()}>
            {t("settings.video.reinstall")}
          </Button>
          <Button variant="ghost" disabled={busy !== null} onclick={confirmRemove}>
            {t("settings.video.remove")}
          </Button>
        {/if}
      </div>
    </div>
  </SettingRow>
{/if}

{#if shown.has("video_channel")}
  <SettingRow
    id="video_channel"
    label={t("settings.video.channel")}
    description={t("settings.video.channel.description")}
    target={highlighted === "video_channel"}
    problem={problems.video_channel}
  >
    {#snippet control()}
      <Select
        value={info?.channel ?? "stable"}
        aria-labelledby="setting-video_channel"
        aria-describedby="setting-video_channel-description"
        disabled={!info}
        options={[
          { value: "stable", label: t("settings.video.channel.stable") },
          { value: "nightly", label: t("settings.video.channel.nightly") },
        ]}
        onchange={(e) => void setChannel(e.currentTarget.value as MediaChannel)}
      />
    {/snippet}
  </SettingRow>
{/if}

{#if shown.has("video_container")}
  <SettingRow
    id="video_container"
    label={t("settings.video.container")}
    description={t("settings.video.container.description")}
    target={highlighted === "video_container"}
    problem={problems.video_container}
  >
    {#snippet control()}
      <Switch
        checked={video.container === "mp4"}
        aria-labelledby="setting-video_container"
        aria-describedby="setting-video_container-description"
        onchange={(e) => {
          const input = e.currentTarget;
          const container = input.checked ? "mp4" : "original";
          void save("video_container", { video: { container } }).then((saved) => {
            if (!saved) input.checked = !input.checked;
          });
        }}
      />
    {/snippet}
  </SettingRow>
{/if}

{#if shown.has("video_paths")}
  <SettingRow
    id="video_paths"
    label={t("settings.video.paths")}
    description={t("settings.video.paths.description")}
    target={highlighted === "video_paths"}
  >
    <div class="paths">
      {#each ["ytDlp", "ffmpeg"] as const as tool (tool)}
        {@const field = `path_${tool}`}
        {@const name = toolName(tool)}
        <label class="path-label" for="video-path-{tool}">{name}</label>
        <div class="path">
          <TextField
            id="video-path-{tool}"
            mono
            bind:value={paths[tool]}
            placeholder={t("settings.video.path.placeholder")}
            aria-label={t("settings.video.path.label", { name })}
            aria-invalid={!!problems[field]}
            aria-describedby={problems[field] ? `video-path-${tool}-problem` : undefined}
            invalid={!!problems[field]}
            spellcheck={false}
            autocomplete="off"
            onfocus={() => (editing = tool)}
            onblur={() => void commitPath(tool)}
            onkeydown={(e) => onPathKey(e, tool)}
          />
          {#if (tool === "ytDlp" ? video.ytDlpPath : video.ffmpegPath) !== null}
            <IconButton label={t("settings.video.path.clear")} onclick={() => clearPath(tool)}>
              <Icon name="close" />
            </IconButton>
          {/if}
        </div>
        {#if problems[field]}
          <p class="problem" id="video-path-{tool}-problem" role="alert">{problems[field]}</p>
        {/if}
      {/each}
    </div>
  </SettingRow>
{/if}

<style>
  .tools {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .lines {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .lines li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-variant-numeric: tabular-nums;
  }
  .lines li :global(.icon) {
    color: var(--color-text-subtle);
  }
  .name {
    min-width: 72px;
    font-weight: var(--weight-medium);
  }
  .note {
    margin: 0;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .paths {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }
  .path-label {
    font-weight: var(--weight-medium);
  }
  .path {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
  }
  .path :global(.text-field) {
    flex: 1;
  }
  .problem {
    grid-column: 2;
    margin: 0;
    color: var(--color-error);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
</style>
