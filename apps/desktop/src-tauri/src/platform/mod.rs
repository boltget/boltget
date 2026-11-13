//! Platform code. On Windows: DWM attributes, WebView2 settings, the native
//! window over the maximize button that keeps snap layouts
//! (docs/APP-DESIGN.md #29 to #31), what the tray needs to know about the
//! system, and the settings that live in Windows (the system proxy, the Run
//! key, the clipboard listener), and a finished queue's sleep or shut down.
//! Elsewhere these are no-ops for now; macOS and Linux come after 1.0
//! (TECH-STACK.md, ADR-016).

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use self::windows::{
    ClipboardWatch, animations_enabled, autostart_enabled, install, set_app_icon, set_autostart,
    set_caption_layout, set_chrome, set_low_memory, shut_down, sleep, small_icon_size,
    system_proxy, taskbar_is_light, update_caption_layout, watch_clipboard, watch_system_proxy,
    watch_taskbar_theme,
};

#[cfg(not(windows))]
mod other {
    use tauri::WebviewWindow;

    use crate::commands::CaptionRect;
    use crate::settings::SystemProxy;

    pub(crate) fn install(_window: &WebviewWindow, _dark: bool) {}
    pub(crate) fn set_chrome(_window: &WebviewWindow, _dark: bool, _border: [u8; 3]) {}
    pub(crate) fn set_caption_layout(_window: &WebviewWindow, _maximize: CaptionRect) {}
    pub(crate) fn update_caption_layout(_window: &WebviewWindow) {}
    pub(crate) fn set_low_memory(_window: &WebviewWindow, _low: bool) {}
    /// The window keeps the icon Tauri gives it (Onyx).
    pub(crate) fn set_app_icon(_window: &WebviewWindow, _light: bool) {}
    pub(crate) fn small_icon_size() -> u32 {
        32
    }
    pub(crate) fn animations_enabled() -> bool {
        true
    }

    /// No taskbar colour is read yet: the tray mark is white, as on a
    /// dark panel.
    pub(crate) fn taskbar_is_light() -> bool {
        false
    }

    pub(crate) fn watch_taskbar_theme(changed: impl Fn() + Send + 'static) {
        drop(changed);
    }

    /// No system proxy is read yet: direct.
    pub(crate) fn system_proxy() -> SystemProxy {
        SystemProxy::default()
    }

    pub(crate) fn watch_system_proxy(changed: impl Fn() + Send + 'static) {
        drop(changed);
    }

    pub(crate) fn autostart_enabled() -> bool {
        false
    }

    pub(crate) fn set_autostart(enabled: bool) -> Result<(), String> {
        if enabled {
            Err("starting with the system is supported on Windows only".to_owned())
        } else {
            Ok(())
        }
    }

    /// Clipboard monitoring is Windows-only for now: there is never a
    /// watch to hold.
    pub(crate) type ClipboardWatch = std::convert::Infallible;

    pub(crate) fn watch_clipboard(
        on_text: impl Fn(String) + Send + 'static,
    ) -> Option<ClipboardWatch> {
        drop(on_text);
        None
    }

    /// A finished queue's sleep: Windows-only for now.
    pub(crate) fn sleep() -> Result<(), String> {
        Err("sleeping after a queue is supported on Windows only".to_owned())
    }

    /// A finished queue's shut down: Windows-only for now.
    pub(crate) fn shut_down() -> Result<(), String> {
        Err("shutting down after a queue is supported on Windows only".to_owned())
    }
}
#[cfg(not(windows))]
pub(crate) use self::other::{
    ClipboardWatch, animations_enabled, autostart_enabled, install, set_app_icon, set_autostart,
    set_caption_layout, set_chrome, set_low_memory, shut_down, sleep, small_icon_size,
    system_proxy, taskbar_is_light, update_caption_layout, watch_clipboard, watch_system_proxy,
    watch_taskbar_theme,
};
