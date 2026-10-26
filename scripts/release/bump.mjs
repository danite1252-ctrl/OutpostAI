#!/usr/bin/env node
// Set Outpost's version everywhere it's written: `node scripts/release/bump.mjs 0.2.0`.
// The phone app's versionCode goes up by one each time (Android needs it to grow).
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(version ?? "")) {
  console.error("Usage: node scripts/release/bump.mjs 1.2.3");
  process.exit(1);
}
const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const edit = (file, fn) => {
  const path = join(root, file);
  const before = readFileSync(path, "utf8");
  const after = fn(before);
  if (after === before) throw new Error(`Nothing changed in ${file}: is the version already ${version}?`);
  writeFileSync(path, after);
  console.log(`  ${file}`);
};
const json = (file, fn) =>
  edit(file, (text) => {
    const j = JSON.parse(text);
    fn(j);
    return JSON.stringify(j, null, 2) + "\n";
  });

console.log(`Setting Outpost's version to ${version}:`);
// The Rust crates (gateway, engines, core, desktop shell) share the workspace version.
edit("Cargo.toml", (t) => t.replace(/(\[workspace\.package\][^[]*?\nversion = ")[^"]+(")/, `$1${version}$2`));
json("apps/desktop/package.json", (j) => (j.version = version));
json("apps/desktop/src-tauri/tauri.conf.json", (j) => (j.version = version));
json("apps/mobile/package.json", (j) => (j.version = version));
let code = 0;
json("apps/mobile/app.json", (j) => {
  j.expo.version = version;
  j.expo.android.versionCode = code = (j.expo.android.versionCode ?? 0) + 1;
});
edit("apps/mobile/src/version.ts", (t) => t.replace(/APP_VERSION = "[^"]*"/, `APP_VERSION = "${version}"`));
console.log(`Done (phone app versionCode ${code}). Cargo.lock updates on the next build.`);
