// The words for each connection state (BROWSER-INTEGRATION.md, "Connection status").
import type { ConnectionStatus } from "../native";
import { t } from "../i18n";

/** How a state reads: a title, a hint, and whether all is well. */
export interface StatusText {
  title: string;
  hint: string;
  tone: "ok" | "idle" | "problem";
  /** Offer the download page. */
  getApp: boolean;
}

export function statusText(status: ConnectionStatus | null): StatusText {
  if (status === null) {
    return { title: t("status.checking"), hint: "", tone: "idle", getApp: false };
  }
  switch (status.state) {
    case "connected":
      return {
        title: t("status.connected", { app: status.app }),
        hint: t("status.connected_hint"),
        tone: "ok",
        getApp: false,
      };
    case "not_running":
      return {
        title: t("status.not_running"),
        hint: t("status.not_running_hint"),
        tone: "idle",
        getApp: false,
      };
    case "not_installed":
      return {
        title: t("status.not_installed"),
        hint: t("status.not_installed_hint"),
        tone: "problem",
        getApp: true,
      };
    case "app_too_old":
      return {
        title: t("status.app_too_old"),
        hint: t("status.app_too_old_hint"),
        tone: "problem",
        getApp: true,
      };
    case "extension_too_old":
      return {
        title: t("status.extension_too_old"),
        hint: t("status.extension_too_old_hint"),
        tone: "problem",
        getApp: false,
      };
    case "mismatch":
      return {
        title: t("status.mismatch"),
        hint: t("status.mismatch_hint"),
        tone: "problem",
        getApp: true,
      };
    case "unreachable":
      return {
        title: t("status.unreachable"),
        hint: t("status.unreachable_hint"),
        tone: "problem",
        getApp: false,
      };
  }
}
