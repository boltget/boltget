// The modifier-key watcher (docs/BROWSER-INTEGRATION.md, "Modifier-key
// bypass"). Registered at run time, and only while the user has given the
// extension site access and kept the setting on; it reads nothing on the page
// and only reports which keys were held when a link was activated.
//
// An unlisted script, not a WXT content script: WXT would add the content
// script's match patterns to the manifest's required host permissions, and
// site access must stay optional. The background registers it with
// `scripting.registerContentScripts`.
import { browser } from "wxt/browser";
import { defineUnlistedScript } from "wxt/utils/define-unlisted-script";
import type { LinkClick } from "../lib/modifiers";

export default defineUnlistedScript({
  main() {
    function report(event: MouseEvent | KeyboardEvent) {
      if (!event.altKey) return;
      const target = event.target instanceof Element ? event.target : null;
      const link = target?.closest<HTMLAnchorElement | HTMLAreaElement>("a[href], area[href]");
      if (!link) return;
      const message: LinkClick = {
        type: "boltget:click",
        href: link.href,
        alt: event.altKey,
        shift: event.shiftKey,
      };
      browser.runtime.sendMessage(message).catch(() => {
        // The extension was updated or removed: nothing to tell.
      });
    }
    // mousedown comes before the click that starts the download.
    document.addEventListener("mousedown", (e) => e.button === 0 && report(e), true);
    document.addEventListener("keydown", (e) => e.key === "Enter" && report(e), true);
  },
});
