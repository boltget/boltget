// Generates src/lib/bindings.ts from the Rust commands and types (the
// `bindings_are_up_to_date` test in src-tauri/src/ipc.rs does the work).
//
//   node scripts/bindings.mjs           write the bindings
//   node scripts/bindings.mjs --check   fail if the committed file is stale (CI)
import { spawnSync } from "node:child_process";

const check = process.argv.includes("--check");
const env = { ...process.env };
if (check) delete env.BOLTGET_WRITE_BINDINGS;
else env.BOLTGET_WRITE_BINDINGS = "1";

const result = spawnSync(
  "cargo",
  [
    "test",
    "--package",
    "boltget-desktop",
    "--locked",
    "--",
    "ipc::tests::bindings_are_up_to_date",
    "--exact",
  ],
  { stdio: "inherit", env },
);
if (result.error) throw result.error;
process.exit(result.status ?? 1);
