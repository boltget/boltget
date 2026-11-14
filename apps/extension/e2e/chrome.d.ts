// The extension APIs inside `worker.evaluate(...)`, which runs in the
// extension's service worker where `chrome` is a global.
import type { browser } from "wxt/browser";

declare global {
  const chrome: typeof browser;
}

export {};
