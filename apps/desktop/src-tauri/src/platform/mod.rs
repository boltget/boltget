//! Platform window chrome. On Windows: DWM attributes, WebView2 settings,
//! the native window over the maximize button that keeps snap layouts
//! (docs/APP-DESIGN.md #29 to #31), and what the tray needs to know about
//! the system. Elsewhere these are no-ops for now; macOS
//! and Linux come after 1.0 (TECH-STACK.md, ADR-016).

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use self::windows::{
    animations_enabled, install, set_caption_layout, set_chrome, small_icon_size,
    update_caption_layout,
};

#[cfg(not(windows))]
mod other {
    use tauri::WebviewWindow;

    use crate::commands::CaptionRect;

    pub(crate) fn install(_window: &WebviewWindow, _dark: bool) {}
    pub(crate) fn set_chrome(_window: &WebviewWindow, _dark: bool, _border: [u8; 3]) {}
    pub(crate) fn set_caption_layout(_window: &WebviewWindow, _maximize: CaptionRect) {}
    pub(crate) fn update_caption_layout(_window: &WebviewWindow) {}
    pub(crate) fn small_icon_size() -> u32 {
        32
    }
    pub(crate) fn animations_enabled() -> bool {
        true
    }
}
#[cfg(not(windows))]
pub(crate) use self::other::{
    animations_enabled, install, set_caption_layout, set_chrome, small_icon_size,
    update_caption_layout,
};
