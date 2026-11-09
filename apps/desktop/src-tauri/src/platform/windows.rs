//! Windows 11 window chrome for the custom title bar (docs/APP-DESIGN.md #29
//! to #31).
//!
//! - **Snap layouts.** The window has no system title bar, and the WebView
//!   covers the whole client area, so Windows never learns where the maximize
//!   button is and cannot show the snap layout flyout over it. A small native
//!   child window sits over the page's maximize button and answers the hit test
//!   with `HTMAXBUTTON` (the approach of Tauri issue #4531 and of Windows
//!   Terminal's drag bar). It forwards hover and press to the page, which draws
//!   the button, and maximizes or restores on click. It paints nothing: the
//!   WebView's content is composed above the window's own surface.
//! - **DWM:** rounded corners, the border colour and dark mode per theme.
//! - **WebView2:** no browser context menu, status bubble, pinch zoom or swipe
//!   navigation; no browser accelerator keys (reload, find, print, devtools)
//!   in release builds.
//! - **The tray:** the system's small icon size, whether Windows shows
//!   animations (the tray mark stays still when it does not), and whether the
//!   taskbar is light (the mark's colour), read and watched in the registry.
//!
//! All of this runs on the main (UI) thread, which owns the windows.
//!
//! The settings that live in Windows are here too, each on a thread of its
//! own: the proxy Windows uses (read with `WinHTTP`, watched in the
//! registry), the Run key that starts Boltget at sign-in, and the clipboard
//! listener (a message-only window).
#![expect(
    unsafe_code,
    reason = "Win32, DWM and WebView2 FFI; every unsafe block states why it is sound"
)]

use std::cell::Cell;
use std::ffi::c_void;
use std::fmt::Display;
use std::sync::{Mutex, Once, PoisonError};

use tauri::WebviewWindow;
use tauri_specta::Event as _;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ,
    COREWEBVIEW2_PERMISSION_STATE_ALLOW, COREWEBVIEW2_PERMISSION_STATE_DENY, ICoreWebView2_19,
    ICoreWebView2Controller, ICoreWebView2Settings3, ICoreWebView2Settings5,
    ICoreWebView2Settings6,
};
use webview2_com::PermissionRequestedEventHandler;
use windows::Win32::Foundation::{
    COLORREF, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DWMWINDOWATTRIBUTE, DwmSetWindowAttribute,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, FindWindowExW, GWLP_USERDATA, GetClientRect,
    GetParent, GetSystemMetrics, GetWindowLongPtrW, HTMAXBUTTON, HWND_TOP, IDC_ARROW, IsZoomed,
    LoadCursorW, PostMessageW, RegisterClassExW, SC_MAXIMIZE, SC_RESTORE, SM_CXSMICON,
    SPI_GETCLIENTAREAANIMATION, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowLongPtrW, SetWindowPos, SystemParametersInfoW,
    WINDOW_EX_STYLE, WM_ERASEBKGND, WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
    WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SYSCOMMAND, WNDCLASSEXW,
    WS_CHILD, WS_CLIPSIBLINGS,
};
use windows::core::{Interface, PCWSTR, w};

use crate::commands::{CaptionMaximize, CaptionRect};
use crate::settings::SystemProxy;

/// `HTMAXBUTTON` as an `LRESULT` value.
#[expect(clippy::cast_possible_wrap, reason = "HTMAXBUTTON is 9")]
const HIT_MAXIMIZE: isize = HTMAXBUTTON as isize;
const OVERLAY_CLASS: PCWSTR = w!("BoltgetCaptionMaximize");
/// Tauri's own child window for resizing an undecorated window from its top
/// edge. The overlay stays below it, so the top edge still resizes.
const TAURI_RESIZE_CLASS: PCWSTR = w!("TAURI_DRAG_RESIZE_BORDERS");

/// The overlay window and where the page last said the button is.
#[derive(Debug, Clone, Copy)]
struct Caption {
    /// The overlay's `HWND`, stored as an integer so the state is `Send`.
    overlay: isize,
    rect: Option<CaptionRect>,
}

static CAPTION: Mutex<Caption> = Mutex::new(Caption {
    overlay: 0,
    rect: None,
});

fn caption() -> Caption {
    *CAPTION.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Prepares the hidden main window: DWM attributes, WebView2 settings and the
/// snap layout overlay.
pub(crate) fn install(window: &WebviewWindow, dark: bool) {
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    set_attribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND);
    set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &i32::from(dark));
    configure_webview(window);
    create_overlay(window, hwnd);
}

/// Applies the page's theme to what the system draws: dark mode and the
/// one-pixel window border.
pub(crate) fn set_chrome(window: &WebviewWindow, dark: bool, border: [u8; 3]) {
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let [r, g, b] = border;
    // COLORREF is 0x00BBGGRR.
    let colour = COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16));
    set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &i32::from(dark));
    set_attribute(hwnd, DWMWA_BORDER_COLOR, &colour);
}

/// Records where the page's maximize button is and moves the overlay there.
pub(crate) fn set_caption_layout(window: &WebviewWindow, maximize: CaptionRect) {
    CAPTION.lock().unwrap_or_else(PoisonError::into_inner).rect = Some(maximize);
    update_caption_layout(window);
}

/// Moves the overlay over the maximize button, in physical pixels for the
/// window's current size and DPI. Called on every resize and DPI change.
pub(crate) fn update_caption_layout(window: &WebviewWindow) {
    let Caption {
        overlay,
        rect: Some(rect),
    } = caption()
    else {
        return;
    };
    let (Ok(parent), false) = (window.hwnd(), overlay == 0) else {
        return;
    };
    let overlay = HWND(overlay as *mut c_void);

    let mut client = RECT::default();
    // SAFETY: `parent` is the live main window and `client` a valid out pointer.
    if unsafe { GetClientRect(parent, &raw mut client) }.is_err() {
        return;
    }
    // SAFETY: `parent` is a live window; the call only reads its DPI.
    let scale = f64::from(unsafe { GetDpiForWindow(parent) }.max(96)) / 96.0;
    let px = |css: f64| (css * scale).round() as i32;

    let right = client.right - px(rect.right);
    let left = client.right - px(rect.right + rect.width);
    let top = px(rect.top);
    let bottom = px(rect.top + rect.height);

    // SAFETY: both windows are children of the live `parent`; a failed lookup
    // yields an error, and HWND_TOP is then a valid insertion point.
    let after = unsafe { FindWindowExW(Some(parent), None, TAURI_RESIZE_CLASS, PCWSTR::null()) }
        .unwrap_or(HWND_TOP);
    // SAFETY: `overlay` is our child window, created on this thread and alive
    // for as long as its parent; the flags neither activate nor reorder others.
    let moved = unsafe {
        SetWindowPos(
            overlay,
            Some(after),
            left,
            top,
            right - left,
            bottom - top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    };
    if let Err(error) = moved {
        warn("could not move the caption overlay", error);
    }
}

/// Sets one DWM window attribute. Failures are ignored: older builds of
/// Windows 10 lack some attributes, and the window works without them.
fn set_attribute<T>(hwnd: HWND, attribute: DWMWINDOWATTRIBUTE, value: &T) {
    // SAFETY: `value` points to a live `T` for the duration of the call, and the
    // size passed is exactly its size; DWM copies the value.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attribute,
            std::ptr::from_ref(value).cast(),
            size_of::<T>() as u32,
        )
    };
}

/// Turns off the browser behaviour a desktop app must not have (#30).
fn configure_webview(window: &WebviewWindow) {
    let release = !cfg!(debug_assertions);
    let queued = window.with_webview(move |webview| {
        // SAFETY: Tauri runs this on the main thread with the webview's live
        // controller; see `apply_settings`.
        if let Err(error) = unsafe { apply_settings(&webview.controller(), release) } {
            warn("could not configure WebView2", error);
        }
    });
    if let Err(error) = queued {
        warn("could not reach the webview", error);
    }
}

/// Asks WebView2 to use as little memory as it can while the window is
/// hidden in the tray, and to return to normal when it is shown
/// (docs/APP-PERFORMANCE.md). The page keeps running either way.
pub(crate) fn set_low_memory(window: &WebviewWindow, low: bool) {
    let queued = window.with_webview(move |webview| {
        // SAFETY: Tauri runs this on the main thread with the webview's live
        // controller; see `apply_memory_target`.
        if let Err(error) = unsafe { apply_memory_target(&webview.controller(), low) } {
            warn("could not set the WebView2 memory target", error);
        }
    });
    if let Err(error) = queued {
        warn("could not reach the webview", error);
    }
}

/// # Safety
///
/// Must be called on the thread that created `controller`, while it is alive.
unsafe fn apply_memory_target(
    controller: &ICoreWebView2Controller,
    low: bool,
) -> windows::core::Result<()> {
    let level = if low {
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
    } else {
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
    };
    // SAFETY: per this function's contract; one property on the live
    // CoreWebView2, through an interface every supported runtime has.
    unsafe {
        controller
            .CoreWebView2()?
            .cast::<ICoreWebView2_19>()?
            .SetMemoryUsageTargetLevel(level)
    }
}

/// # Safety
///
/// Must be called on the thread that created `controller`, while it is alive.
unsafe fn apply_settings(
    controller: &ICoreWebView2Controller,
    release: bool,
) -> windows::core::Result<()> {
    // No browser permission prompt ever appears in the app. Reading the
    // clipboard is allowed: Ctrl+N and "Add download" read it when the
    // user asks (APP-DESIGN #6); without this, WebView2 asks the user
    // whether "tauri.localhost" may see the clipboard. Everything else
    // (camera, location, notifications, ...) is refused: the page needs
    // none of it, and native notifications come from Rust.
    let handler = PermissionRequestedEventHandler::create(Box::new(|_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
        // SAFETY: `args` is the live event argument WebView2 passed in.
        unsafe {
            args.PermissionKind(&raw mut kind)?;
            args.SetState(if kind == COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ {
                COREWEBVIEW2_PERMISSION_STATE_ALLOW
            } else {
                COREWEBVIEW2_PERMISSION_STATE_DENY
            })
        }
    }));

    // SAFETY: per this function's contract; each call sets one boolean on a
    // settings object the wrappers keep referenced, and the handler is
    // registered on the live CoreWebView2, which keeps it referenced.
    unsafe {
        let settings = controller.CoreWebView2()?.Settings()?;
        settings.SetAreDefaultContextMenusEnabled(false)?;
        settings.SetIsStatusBarEnabled(false)?;
        if release {
            settings
                .cast::<ICoreWebView2Settings3>()?
                .SetAreBrowserAcceleratorKeysEnabled(false)?;
        }
        settings
            .cast::<ICoreWebView2Settings5>()?
            .SetIsPinchZoomEnabled(false)?;
        settings
            .cast::<ICoreWebView2Settings6>()?
            .SetIsSwipeNavigationEnabled(false)?;

        let mut token = 0;
        controller
            .CoreWebView2()?
            .add_PermissionRequested(&handler, &raw mut token)?;
    }
    Ok(())
}

/// How the page should draw its maximize button; also the event payload.
type ButtonState = CaptionMaximize;

/// Per-window data of the overlay, owned by the window (`GWLP_USERDATA`)
/// from `WM_NCCREATE` until `WM_NCDESTROY`.
struct Overlay {
    state: Cell<ButtonState>,
    tracking: Cell<bool>,
    notify: Box<dyn Fn(ButtonState)>,
}

impl Overlay {
    fn set(&self, state: ButtonState) {
        if self.state.replace(state) != state {
            (self.notify)(state);
        }
    }

    /// Handles a message, or returns `None` for default processing.
    fn handle(&self, hwnd: HWND, msg: u32) -> Option<LRESULT> {
        match msg {
            WM_NCMOUSEMOVE => {
                if !self.tracking.get() {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: `track` is a valid, initialised TRACKMOUSEEVENT
                    // for our own window.
                    self.tracking
                        .set(unsafe { TrackMouseEvent(&raw mut track) }.is_ok());
                }
                if self.state.get() == ButtonState::Rest {
                    self.set(ButtonState::Hover);
                }
                // Default processing: this is where Windows 11 opens the snap
                // layout flyout after a short hover.
                None
            }
            WM_NCMOUSELEAVE => {
                self.tracking.set(false);
                self.set(ButtonState::Rest);
                Some(LRESULT(0))
            }
            // Not passed on: DefWindowProc would run its own caption-button
            // loop and draw a classic button over the page.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
                self.set(ButtonState::Press);
                Some(LRESULT(0))
            }
            WM_NCLBUTTONUP => {
                if self.state.get() == ButtonState::Press {
                    toggle_maximized(hwnd);
                }
                self.set(ButtonState::Hover);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

/// Maximizes or restores the overlay's parent, as its maximize button would.
fn toggle_maximized(overlay: HWND) {
    // SAFETY: `overlay` is our live child window; GetParent only reads it.
    let Ok(parent) = (unsafe { GetParent(overlay) }) else {
        return;
    };
    // SAFETY: `parent` is the live main window; IsZoomed only reads its state.
    let command = if unsafe { IsZoomed(parent) }.as_bool() {
        SC_RESTORE
    } else {
        SC_MAXIMIZE
    };
    // SAFETY: posting a standard system command to a live window; the message
    // carries no pointers.
    let posted = unsafe {
        PostMessageW(
            Some(parent),
            WM_SYSCOMMAND,
            WPARAM(command as usize),
            LPARAM(0),
        )
    };
    if let Err(error) = posted {
        warn("could not maximize the window", error);
    }
}

fn create_overlay(window: &WebviewWindow, parent: HWND) {
    static REGISTER: Once = Once::new();

    // SAFETY: GetModuleHandleW(None) returns this executable's handle and
    // takes no ownership.
    let instance = unsafe { GetModuleHandleW(None) }
        .map(|module| HINSTANCE(module.0))
        .unwrap_or_default();

    REGISTER.call_once(|| {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(overlay_proc),
            hInstance: instance,
            // SAFETY: loading a predefined system cursor.
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            lpszClassName: OVERLAY_CLASS,
            ..Default::default()
        };
        // SAFETY: `class` is fully initialised and its strings are static.
        if unsafe { RegisterClassExW(&raw const class) } == 0 {
            warn("could not register the caption overlay", "RegisterClassExW");
        }
    });

    let page = window.clone();
    let overlay = Box::new(Overlay {
        state: Cell::new(ButtonState::Rest),
        tracking: Cell::new(false),
        notify: Box::new(move |state| {
            let _ = state.emit(&page);
        }),
    });
    let data = Box::into_raw(overlay);

    // SAFETY: `parent` is the live main window on this thread and the class is
    // registered. Ownership of `data` passes to the window in WM_NCCREATE and
    // is reclaimed in WM_NCDESTROY. If creation fails before WM_NCCREATE the
    // box leaks once, which is harmless; freeing it here could double-free.
    let created = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            OVERLAY_CLASS,
            PCWSTR::null(),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            Some(parent),
            None,
            Some(instance),
            Some(data.cast::<c_void>().cast_const()),
        )
    };
    match created {
        Ok(hwnd) => {
            CAPTION
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .overlay = hwnd.0 as isize;
        }
        Err(error) => warn("could not create the caption overlay", error),
    }
}

/// The overlay's data, if it is still attached to the window.
///
/// # Safety
///
/// `hwnd` must be an overlay window created by [`create_overlay`], and the
/// call must be on its thread.
unsafe fn overlay_data<'a>(hwnd: HWND) -> Option<&'a Overlay> {
    // SAFETY: per this function's contract, GWLP_USERDATA is null or the
    // pointer stored in WM_NCCREATE, valid until WM_NCDESTROY clears it.
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Overlay).as_ref() }
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            // SAFETY: for WM_NCCREATE, `lparam` points to this window's
            // CREATESTRUCTW, whose lpCreateParams is the `Overlay` box pointer
            // passed to CreateWindowExW.
            unsafe {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
        }
        WM_NCDESTROY => {
            // SAFETY: WM_NCDESTROY is the last message this window receives;
            // the pointer is detached first, so the box is freed exactly once.
            unsafe {
                let data = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Overlay;
                if !data.is_null() {
                    drop(Box::from_raw(data));
                }
            }
        }
        // The whole overlay is the maximize button.
        WM_NCHITTEST => return LRESULT(HIT_MAXIMIZE),
        // Paint nothing; the page draws the button.
        WM_ERASEBKGND => return LRESULT(1),
        _ => {
            // SAFETY: this procedure only serves overlay windows, on their thread.
            if let Some(result) = unsafe { overlay_data(hwnd) }.and_then(|o| o.handle(hwnd, msg)) {
                return result;
            }
        }
    }
    // SAFETY: default processing with the arguments we were given.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// The system's small icon size in pixels (the tray's), at the primary
/// monitor's DPI.
pub(crate) fn small_icon_size() -> u32 {
    // SAFETY: GetSystemMetrics has no preconditions.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
    u32::try_from(size).ok().filter(|&s| s >= 16).unwrap_or(16)
}

/// Whether Windows shows animations ("Animation effects" in Settings);
/// when it does not, motion is reduced.
pub(crate) fn animations_enabled() -> bool {
    let mut enabled = windows::core::BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer,
    // which points at `enabled`.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || enabled.as_bool()
}

/// Window chrome failures are cosmetic: report them in debug builds only.
fn warn(what: &str, error: impl Display) {
    if cfg!(debug_assertions) {
        eprintln!("boltget-desktop: {what}: {error}");
    }
}

// ---------------------------------------------------------------------------
// Settings that live in Windows (docs/APP-DESIGN.md, "Settings as built"):
// the proxy Windows is set to use, starting with Windows, and the clipboard.
// ---------------------------------------------------------------------------

/// Where `WinINet` keeps the proxy settings (and its per-connection blobs).
const INTERNET_SETTINGS: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
/// The current user's Run key, which starts programs at sign-in.
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
/// The value Boltget uses in the Run key.
const RUN_VALUE: PCWSTR = w!("Boltget");

/// Takes a string `WinHTTP` allocated: its text, and frees it.
///
/// # Safety
///
/// `text` must be null or a NUL-terminated string allocated with
/// `GlobalAlloc` that nothing else frees.
unsafe fn take_global_string(text: windows::core::PWSTR) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: per the contract, `text` is a live NUL-terminated string.
    let value = unsafe { text.to_string() }.ok();
    // SAFETY: per the contract, we own the allocation and free it once.
    let _ = unsafe { windows::Win32::Foundation::GlobalFree(Some(HGLOBAL(text.0.cast()))) };
    value.filter(|v| !v.trim().is_empty())
}

/// The proxy Windows is set to use for the current user (Settings >
/// Network & internet > Proxy), as `WinINet` keeps it.
pub(crate) fn system_proxy() -> SystemProxy {
    use windows::Win32::Networking::WinHttp::{
        WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WinHttpGetIEProxyConfigForCurrentUser,
    };
    let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    // SAFETY: `config` is a valid out pointer; on success its strings are
    // ours to free with GlobalFree, which `take_global_string` does.
    if let Err(error) = unsafe { WinHttpGetIEProxyConfigForCurrentUser(&raw mut config) } {
        tracing::warn!(%error, "could not read the Windows proxy settings");
        return SystemProxy::default();
    }
    // SAFETY: each string comes from the call above and is taken once.
    unsafe {
        SystemProxy {
            server: take_global_string(config.lpszProxy),
            bypass: take_global_string(config.lpszProxyBypass),
            script: take_global_string(config.lpszAutoConfigUrl),
        }
    }
}

/// Calls `changed` (on a thread of its own) after the Windows proxy
/// settings change, for as long as the app runs. A burst of changes is read
/// once, half a second after it starts.
pub(crate) fn watch_system_proxy(changed: impl Fn() + Send + 'static) {
    watch_key(
        "boltget-proxy-watch",
        INTERNET_SETTINGS,
        "the Windows proxy settings",
        changed,
    );
}

/// Where Windows keeps the current user's light and dark mode choices.
const PERSONALIZE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

/// Whether the taskbar (and the tray on it) is light: Settings >
/// Personalization > Colors, "Choose your default Windows mode". Only read,
/// never written. Windows 10 before 1903 has no such value: dark.
pub(crate) fn taskbar_is_light() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let mut value = 0u32;
    let mut bytes = 4u32;
    // SAFETY: `value` has room for the `bytes` (4) bytes of a REG_DWORD,
    // which is all RRF_RT_REG_DWORD lets RegGetValueW write.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE,
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&raw mut bytes),
        )
    };
    read.is_ok() && value != 0
}

/// Calls `changed` (on a thread of its own) after the light or dark mode
/// choices change, for as long as the app runs.
pub(crate) fn watch_taskbar_theme(changed: impl Fn() + Send + 'static) {
    watch_key(
        "boltget-theme-watch",
        PERSONALIZE,
        "the Windows colour mode",
        changed,
    );
}

/// Calls `changed` on a thread named `thread` after anything under the
/// current user's `subkey` changes, for as long as the app runs. A burst of
/// changes is reported once, half a second after it starts. `what` names
/// the settings in warnings.
fn watch_key(
    thread: &str,
    subkey: PCWSTR,
    what: &'static str,
    changed: impl Fn() + Send + 'static,
) {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RegCloseKey,
        RegNotifyChangeKeyValue, RegOpenKeyExW,
    };
    // PCWSTR is not Send; the `w!` names it points at are static.
    let subkey = subkey.0 as usize;
    let spawned = std::thread::Builder::new()
        .name(thread.into())
        .spawn(move || {
            let mut key = HKEY::default();
            // SAFETY: opening a key of the current user with a static,
            // NUL-terminated name; `key` is a valid out pointer.
            let opened = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(subkey as *const u16),
                    None,
                    KEY_NOTIFY,
                    &raw mut key,
                )
            };
            if opened.is_err() {
                tracing::warn!(error = ?opened, "cannot watch {what}");
                return;
            }
            loop {
                // SAFETY: `key` is open with KEY_NOTIFY; the synchronous call
                // blocks this thread until something under it changes.
                let waited = unsafe {
                    RegNotifyChangeKeyValue(key, true, REG_NOTIFY_CHANGE_LAST_SET, None, false)
                };
                if waited.is_err() {
                    tracing::warn!(error = ?waited, "stopped watching {what}");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
                changed();
            }
            // SAFETY: closing the key opened above, once.
            let _ = unsafe { RegCloseKey(key) };
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot watch {what}");
    }
}

/// The command line the Run key starts: this executable, in the tray.
fn autostart_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --background", exe.display()))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Whether the Run key starts this Boltget at sign-in.
pub(crate) fn autostart_enabled() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let mut buffer = vec![0u16; 1024];
    let mut bytes = u32::try_from(buffer.len() * 2).unwrap_or(u32::MAX);
    // SAFETY: `buffer` has room for `bytes` bytes; RegGetValueW writes at
    // most that many and NUL-terminates REG_SZ data.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut bytes),
        )
    };
    if read.is_err() {
        return false;
    }
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..len]).to_lowercase();
    std::env::current_exe()
        .is_ok_and(|exe| value.contains(&exe.display().to_string().to_lowercase()))
}

/// Adds Boltget to the current user's Run key, or removes it.
pub(crate) fn set_autostart(enabled: bool) -> Result<(), String> {
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
    };
    if !enabled {
        // SAFETY: deleting a named value under a key of the current user,
        // with static names.
        let deleted = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE) };
        return if deleted.is_ok() || deleted == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("RegDeleteKeyValueW failed: {deleted:?}"))
        };
    }
    let command = autostart_command().ok_or("the executable's path is unknown")?;
    let data = wide(&command);
    let bytes = u32::try_from(data.len() * 2).map_err(|e| e.to_string())?;
    // SAFETY: `data` is a NUL-terminated UTF-16 string of `bytes` bytes, as
    // REG_SZ requires, and lives across the call.
    let set = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            bytes,
        )
    };
    if set.is_ok() {
        Ok(())
    } else {
        Err(format!("RegSetKeyValueW failed: {set:?}"))
    }
}

/// Clipboard text longer than this is not a link someone copied.
const CLIPBOARD_MAX_CHARS: usize = 64 * 1024;
const CLIPBOARD_CLASS: PCWSTR = w!("BoltgetClipboardListener");

/// The callback of a clipboard listener thread.
type OnText = Box<dyn Fn(String)>;

thread_local! {
    /// The listener thread's callback; its window procedure runs on it.
    static ON_CLIPBOARD: std::cell::RefCell<Option<OnText>> =
        const { std::cell::RefCell::new(None) };
}

/// Listens for clipboard changes while it lives (Windows' change
/// notification, no polling); dropping it stops listening.
#[derive(Debug)]
pub(crate) struct ClipboardWatch {
    /// The listener's message-only window, as an integer so this is `Send`.
    hwnd: isize,
}

impl Drop for ClipboardWatch {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;
        // SAFETY: posting WM_CLOSE carries no pointers; if the window is
        // already gone the call fails harmlessly.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.hwnd as *mut c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
}

/// Starts listening: `on_text` gets the clipboard's text each time another
/// program puts text there. Text Boltget copied itself, and text marked as
/// private (password managers set
/// `ExcludeClipboardContentFromMonitorProcessing`), is never read.
pub(crate) fn watch_clipboard(on_text: impl Fn(String) + Send + 'static) -> Option<ClipboardWatch> {
    let (tx, rx) = std::sync::mpsc::channel::<isize>();
    let spawned = std::thread::Builder::new()
        .name("boltget-clipboard".into())
        .spawn(move || clipboard_thread(Box::new(on_text), &tx));
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot watch the clipboard");
        return None;
    }
    match rx.recv() {
        Ok(hwnd) if hwnd != 0 => Some(ClipboardWatch { hwnd }),
        _ => None,
    }
}

fn clipboard_thread(on_text: OnText, ready: &std::sync::mpsc::Sender<isize>) {
    use windows::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, WINDOW_STYLE,
    };
    static REGISTER: Once = Once::new();

    // SAFETY: GetModuleHandleW(None) returns this executable's handle and
    // takes no ownership.
    let instance = unsafe { GetModuleHandleW(None) }
        .map(|module| HINSTANCE(module.0))
        .unwrap_or_default();
    REGISTER.call_once(|| {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(clipboard_proc),
            hInstance: instance,
            lpszClassName: CLIPBOARD_CLASS,
            ..Default::default()
        };
        // SAFETY: `class` is fully initialised and its strings are static.
        if unsafe { RegisterClassExW(&raw const class) } == 0 {
            tracing::warn!("could not register the clipboard listener's class");
        }
    });
    ON_CLIPBOARD.with(|cell| *cell.borrow_mut() = Some(on_text));
    // SAFETY: a message-only window of a registered class, owned by this
    // thread, without create parameters.
    let created = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLIPBOARD_CLASS,
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        )
    };
    let hwnd = match created {
        Ok(hwnd) => hwnd,
        Err(error) => {
            tracing::warn!(%error, "could not create the clipboard listener");
            let _ = ready.send(0);
            return;
        }
    };
    // SAFETY: `hwnd` is the live window created above, on this thread.
    if let Err(error) = unsafe { AddClipboardFormatListener(hwnd) } {
        tracing::warn!(%error, "could not listen to the clipboard");
        // SAFETY: destroying the window this thread created.
        let _ = unsafe { DestroyWindow(hwnd) };
        let _ = ready.send(0);
        return;
    }
    let _ = ready.send(hwnd.0 as isize);
    let mut msg = MSG::default();
    // SAFETY: a standard message loop for this thread's window; `msg` is a
    // valid out pointer. It ends with WM_QUIT, posted in WM_DESTROY.
    while unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.as_bool() {
        // SAFETY: dispatching the message just received.
        unsafe { DispatchMessageW(&raw const msg) };
    }
    ON_CLIPBOARD.with(|cell| cell.borrow_mut().take());
}

unsafe extern "system" fn clipboard_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use windows::Win32::System::DataExchange::RemoveClipboardFormatListener;
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, PostQuitMessage, WM_CLIPBOARDUPDATE, WM_CLOSE, WM_DESTROY,
    };
    match msg {
        WM_CLIPBOARDUPDATE => {
            if let Some(text) = read_clipboard_text(hwnd) {
                ON_CLIPBOARD.with(|cell| {
                    if let Some(on_text) = cell.borrow().as_ref() {
                        on_text(text);
                    }
                });
            }
            return LRESULT(0);
        }
        WM_CLOSE => {
            // SAFETY: this thread's own window, still alive.
            unsafe {
                let _ = RemoveClipboardFormatListener(hwnd);
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }
        // SAFETY: ends this thread's message loop.
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ => {}
    }
    // SAFETY: default processing with the arguments we were given.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// The clipboard's text, unless it is ours, private, not text or too long.
fn read_clipboard_text(hwnd: HWND) -> Option<String> {
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, GetClipboardOwner, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    let text_format = u32::from(CF_UNICODETEXT.0);
    // SAFETY: a format query; no pointers.
    if unsafe { IsClipboardFormatAvailable(text_format) }.is_err() {
        return None;
    }
    // SAFETY: registering (or finding) a named format; the name is static.
    let private =
        unsafe { RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")) };
    // SAFETY: a format query; no pointers.
    if private != 0 && unsafe { IsClipboardFormatAvailable(private) }.is_ok() {
        return None;
    }
    // SAFETY: GetClipboardOwner takes no arguments.
    if let Ok(owner) = unsafe { GetClipboardOwner() } {
        let mut process = 0u32;
        // SAFETY: `owner` is a window handle (possibly stale, which the call
        // tolerates); the process id goes to a valid out pointer.
        unsafe { GetWindowThreadProcessId(owner, Some(&raw mut process)) };
        // SAFETY: no arguments.
        if process == unsafe { GetCurrentProcessId() } {
            return None;
        }
    }
    // Another program may hold the clipboard for a moment.
    let mut opened = false;
    for _ in 0..5 {
        // SAFETY: `hwnd` is this thread's live window.
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !opened {
        return None;
    }
    let text = (|| {
        // SAFETY: the clipboard is open; the handle stays owned by it.
        let handle = unsafe { GetClipboardData(text_format) }.ok()?;
        let global = HGLOBAL(handle.0);
        // SAFETY: CF_UNICODETEXT data is an HGLOBAL; locking gives a pointer
        // valid until GlobalUnlock.
        let ptr = unsafe { GlobalLock(global) }.cast::<u16>();
        if ptr.is_null() {
            return None;
        }
        // SAFETY: the size of the locked allocation, in bytes.
        let units = (unsafe { GlobalSize(global) } / 2).min(CLIPBOARD_MAX_CHARS + 1);
        // SAFETY: `ptr` points to at least `units` UTF-16 units while locked.
        let slice = unsafe { std::slice::from_raw_parts(ptr, units) };
        let len = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
        let text = (len <= CLIPBOARD_MAX_CHARS).then(|| String::from_utf16_lossy(&slice[..len]));
        // SAFETY: unlocking what was locked above.
        let _ = unsafe { GlobalUnlock(global) };
        text
    })();
    // SAFETY: closing the clipboard this thread opened.
    let _ = unsafe { CloseClipboard() };
    text
}

// Power: a finished queue's sleep or shut down (src/power.rs) ------------------

/// Enables the shutdown privilege in this process's token, which both
/// sleeping and shutting down need (a normal user has it, disabled).
fn enable_shutdown_privilege() -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID};
    use windows::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = HANDLE::default();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no
    // closing; `token` is a valid place for the handle it opens.
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &raw mut token,
        )
    }
    .map_err(|e| format!("opening the process token: {e}"))?;
    let mut luid = LUID::default();
    // SAFETY: `SE_SHUTDOWN_NAME` is a static, NUL-terminated wide string and
    // `luid` a valid place for the answer.
    let looked_up = unsafe { LookupPrivilegeValueW(None, SE_SHUTDOWN_NAME, &raw mut luid) };
    let result = looked_up
        .map_err(|e| format!("looking up the shutdown privilege: {e}"))
        .and_then(|()| {
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            // SAFETY: `token` was opened with TOKEN_ADJUST_PRIVILEGES above,
            // and `privileges` lives across the call; the previous state is
            // not asked for.
            unsafe {
                AdjustTokenPrivileges(token, false, Some(&raw const privileges), 0, None, None)
            }
            .map_err(|e| format!("enabling the shutdown privilege: {e}"))
        });
    // SAFETY: `token` is a handle this function opened and owns.
    let _ = unsafe { CloseHandle(token) };
    result
}

/// Puts the computer to sleep (not hibernation), as Start's Sleep does.
pub(crate) fn sleep() -> Result<(), String> {
    use windows::Win32::System::Power::SetSuspendState;
    enable_shutdown_privilege()?;
    // SAFETY: plain flags; the call suspends the machine and returns after
    // it wakes up.
    if unsafe { SetSuspendState(false, false, false) } {
        Ok(())
    } else {
        Err(format!(
            "SetSuspendState failed: {}",
            windows::core::Error::from_thread()
        ))
    }
}

/// Shuts the computer down and powers it off, letting apps save their work
/// (no forced close), as a planned shutdown.
pub(crate) fn shut_down() -> Result<(), String> {
    use windows::Win32::System::Shutdown::{
        InitiateShutdownW, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER,
        SHTDN_REASON_MINOR_OTHER, SHUTDOWN_POWEROFF,
    };
    enable_shutdown_privilege()?;
    // SAFETY: no machine name or message (the local machine, no text); the
    // flags and reason are plain values.
    let code = unsafe {
        InitiateShutdownW(
            None,
            None,
            0,
            SHUTDOWN_POWEROFF,
            SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED,
        )
    };
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "InitiateShutdownW failed: {}",
            windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(code))
        ))
    }
}
