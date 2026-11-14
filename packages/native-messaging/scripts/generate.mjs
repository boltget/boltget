// Generates src/protocol.ts from the Rust protocol types (the
// `protocol_types_are_up_to_date` test in crates/native-host does the work).
//
//   node scripts/generate.mjs           write the types
//   node scripts/generate.mjs --check   fail if the committed file is stale
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
    "boltget-native-host",
    "--locked",
    "--test",
    "bindings",
    "--",
    "protocol_types_are_up_to_date",
    "--exact",
  ],
  { stdio: "inherit", env },
);
if (result.error) throw result.error;
process.exit(result.status ?? 1);
