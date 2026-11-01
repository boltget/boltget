//! Platform window chrome. On Windows: DWM attributes, WebView2 settings and
//! the native window over the maximize button that keeps snap layouts
//! (docs/APP-DESIGN.md #29 to #31). Elsewhere these are no-ops for now; macOS
//! and Linux come after 1.0 (TECH-STACK.md, ADR-016).

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use self::windows::{install, set_caption_layout, set_chrome, update_caption_layout};

#[cfg(not(windows))]
mod other {
    use tauri::WebviewWindow;

    use crate::commands::CaptionRect;

    pub(crate) fn install(_window: &WebviewWindow, _dark: bool) {}
    pub(crate) fn set_chrome(_window: &WebviewWindow, _dark: bool, _border: [u8; 3]) {}
    pub(crate) fn set_caption_layout(_window: &WebviewWindow, _maximize: CaptionRect) {}
    pub(crate) fn update_caption_layout(_window: &WebviewWindow) {}
}
#[cfg(not(windows))]
pub(crate) use self::other::{install, set_caption_layout, set_chrome, update_caption_layout};
