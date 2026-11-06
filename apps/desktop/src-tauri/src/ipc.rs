//! The IPC surface in one place: every command and event the page can use,
//! and the TypeScript generated from them (docs/ARCHITECTURE.md, "IPC and
//! events"; the choice of tauri-specta is ADR-017 in docs/TECH-STACK.md).
//!
//! `apps/desktop/src/lib/bindings.ts` is generated, committed, and checked:
//! the `bindings_are_up_to_date` test fails when it is stale, so CI does too.
//! Regenerate it with `pnpm --filter @boltget/desktop bindings`.

use tauri_specta::{Builder, collect_commands, collect_events};

use crate::bridge::commands as engine;
use crate::{clipboard, commands, shell};

/// The generated file, relative to this crate.
#[cfg(test)]
const BINDINGS: &str = "../src/lib/bindings.ts";

/// Commands and events, as Tauri registers them and as the bindings show them.
pub(crate) fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::app_info,
            commands::show_main_window,
            commands::set_window_chrome,
            commands::set_caption_layout,
            commands::quit_app,
            engine::engine_status,
            engine::subscribe_engine,
            engine::list_downloads,
            engine::probe_url,
            engine::add_download,
            engine::add_defaults,
            engine::check_destination,
            engine::choose_folder,
            engine::pause_download,
            engine::resume_download,
            engine::remove_download,
            engine::refresh_link,
            engine::set_speed_limit,
            engine::get_settings,
            engine::update_settings,
            engine::system_proxy,
            engine::open_downloads_folder,
            engine::download_segments,
            engine::download_details,
            engine::open_download,
            engine::show_download_in_folder,
            engine::set_focused_download,
            engine::set_ui_visible,
            shell::take_launch_links,
        ])
        .events(collect_events![
            commands::CaptionMaximize,
            shell::ShellRequest,
            clipboard::ClipboardOffer
        ])
        // Byte counts are u64; as JavaScript numbers they are exact up to
        // 2^53 (8 PiB), far beyond any file. Ids are small integers.
        .dangerously_cast_bigints_to_number()
        // One TypeScript type per Rust type: none of ours serialize and
        // deserialize differently, and a channel's payload must be typed as
        // what Rust sends.
        .disable_serde_phases()
}

/// Renders the bindings as committed: generated, with a header that says
/// how, and LF line endings.
#[cfg(test)]
fn render() -> String {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("bindings.ts");
    builder()
        .export(
            specta_typescript::Typescript::default().header(
                "// Generated from apps/desktop/src-tauri by tauri-specta. Do not edit.\n\
                 // Regenerate: pnpm --filter @boltget/desktop bindings\n",
            ),
            &path,
        )
        .expect("the bindings export");
    std::fs::read_to_string(&path)
        .expect("the exported bindings")
        .replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{BINDINGS, render};

    /// Fails when the committed bindings differ from what the Rust types
    /// generate. `BOLTGET_WRITE_BINDINGS=1` rewrites them instead.
    #[test]
    fn bindings_are_up_to_date() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(BINDINGS);
        let fresh = render();
        if std::env::var_os("BOLTGET_WRITE_BINDINGS").is_some() {
            std::fs::write(&path, &fresh).expect("writing the bindings");
            return;
        }
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            committed == fresh,
            "{} is stale: run `pnpm --filter @boltget/desktop bindings` and commit the result",
            path.display()
        );
    }
}
