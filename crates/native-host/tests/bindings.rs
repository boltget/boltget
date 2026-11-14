//! The TypeScript types of the browser protocol, for the extension:
//! `packages/native-messaging/src/protocol.ts` is generated from
//! `boltget_ipc::browser`, committed, and checked here, so CI fails when it
//! is stale. Regenerate it with `pnpm --filter @boltget/native-messaging
//! generate` (which runs this test with `BOLTGET_WRITE_BINDINGS=1`).

use std::path::Path;

use boltget_ipc::browser::{self, Envelope, ReplyEnvelope};

const BINDINGS: &str = "../../packages/native-messaging/src/protocol.ts";

fn render() -> String {
    let types = specta::Types::default()
        .register::<Envelope>()
        .register::<ReplyEnvelope>();
    let constants = format!(
        "\n/** The protocol version this release speaks. */\n\
         export const VERSION = {};\n\
         /** The oldest version it still serves. */\n\
         export const OLDEST_VERSION = {};\n\
         /** The native messaging host's name in release builds. */\n\
         export const HOST_NAME = \"{}\";\n\
         /** The host's name for development builds and the development extension. */\n\
         export const DEV_HOST_NAME = \"{}\";\n\
         /** The largest message the host accepts, in bytes of JSON. */\n\
         export const MAX_MESSAGE = {};\n\
         /** The request headers the host accepts, lowercase. */\n\
         export const ALLOWED_HEADERS = {:?} as const;\n",
        browser::VERSION,
        browser::OLDEST_VERSION,
        browser::HOST_NAME,
        browser::DEV_HOST_NAME,
        browser::MAX_MESSAGE,
        browser::ALLOWED_HEADERS,
    );
    let ts = specta_typescript::Typescript::default()
        .header(
            "// Generated from crates/ipc/src/browser.rs by the native host's tests. Do not edit.\n\
             // Regenerate: pnpm --filter @boltget/native-messaging generate\n",
        )
        .with_raw(constants)
        .export(&types, specta_serde::Format)
        .expect("the protocol exports to TypeScript");
    ts.replace("\r\n", "\n")
}

#[test]
fn protocol_types_are_up_to_date() {
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
        "{} is stale: run `pnpm --filter @boltget/native-messaging generate` and commit the result",
        path.display()
    );
}
