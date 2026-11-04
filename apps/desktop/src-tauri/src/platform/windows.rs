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
//! - **The tray:** the system's small icon size, and whether Windows shows
//!   animations (the tray mark stays still when it does not).
//!
//! All of this runs on the main (UI) thread, which owns the windows.
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
    ICoreWebView2Controller, ICoreWebView2Settings3, ICoreWebView2Settings5, ICoreWebView2Settings6,
};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
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

/// # Safety
///
/// Must be called on the thread that created `controller`, while it is alive.
unsafe fn apply_settings(
    controller: &ICoreWebView2Controller,
    release: bool,
) -> windows::core::Result<()> {
    // SAFETY: per this function's contract; each call sets one boolean on a
    // settings object the wrappers keep referenced.
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
