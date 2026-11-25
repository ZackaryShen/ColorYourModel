#!/usr/bin/env node
// Version bump — single source of truth is src-tauri/tauri.conf.json.
// vite.config.ts injects __APP_VERSION__ from tauri.conf.json, and the
// self-updater compares installer versions against it, so all copies must
// move together. This script keeps them in lockstep.
//
// Usage:
//   node tools/bump-version.mjs            sync package.json / Cargo.toml /
//                                          Cargo.lock from tauri.conf.json
//   node tools/bump-version.mjs 0.2.0      set the version everywhere first
//   node tools/bump-version.mjs --check    verify all copies agree; exit 1
//                                          (wired into CI as a gate)
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_CONF = join(root, "src-tauri", "tauri.conf.json");
const PACKAGE_JSON = join(root, "package.json");
const CARGO_TOML = join(root, "src-tauri", "Cargo.toml");
const CARGO_LOCK = join(root, "src-tauri", "Cargo.lock");
const PKG_NAME = "color-your-model";
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

const read = (p) => readFileSync(p, "utf8");

// The top-level version key appears before any nested "version" keys in both
// JSON files (verified layout); replace only the first occurrence.
function jsonVersion(text, next) {
  const m = text.match(/"version"\s*:\s*"([^"]+)"/);
  if (!m) throw new Error("no \"version\" key found");
  return { old: m[1], text: text.replace(m[0], `"version": "${next}"`) };
}

// Only the line inside the [package] table — Cargo.toml has dozens of
// `version = "..."` dependency lines that must stay untouched.
function cargoTomlVersion(text, next) {
  const start = text.indexOf("[package]");
  if (start < 0) throw new Error("no [package] section");
  const sectionEnd = text.indexOf("\n[", start + 1);
  const section = text.slice(start, sectionEnd < 0 ? undefined : sectionEnd);
  if (!/^version\s*=\s*"([^"]+)"/m.test(section)) {
    throw new Error("no version line in [package]");
  }
  const replaced = section.replace(/^version\s*=\s*"([^"]+)"/m, `version = "${next}"`);
  return {
    old: section.match(/^version\s*=\s*"([^"]+)"/m)[1],
    text: text.slice(0, start) + replaced + text.slice(start + section.length),
  };
}

// Match the exact `name = "<pkg>"` + following `version = "..."` pair so other
// packages' dependency arrays are never touched.
function cargoLockVersion(text, next) {
  const re = new RegExp(
    `(name = "${PKG_NAME}"\\r?\\nversion = ")([^"]+)(")`
  );
  const m = text.match(re);
  if (!m) throw new Error(`${PKG_NAME} not found in Cargo.lock`);
  return { old: m[2], text: text.replace(re, `$1${next}$3`) };
}

function currentVersions() {
  return {
    "tauri.conf.json": read(TAURI_CONF).match(/"version"\s*:\s*"([^"]+)"/)[1],
    "package.json": read(PACKAGE_JSON).match(/"version"\s*:\s*"([^"]+)"/)[1],
    "Cargo.toml": read(CARGO_TOML)
      .slice(read(CARGO_TOML).indexOf("[package]"))
      .match(/^version\s*=\s*"([^"]+)"/m)[1],
    "Cargo.lock": read(CARGO_LOCK).match(
      new RegExp(`name = "${PKG_NAME}"\\r?\\nversion = "([^"]+)"`)
    )[1],
  };
}

const arg = process.argv[2];

if (arg === "--check") {
  const v = currentVersions();
  const values = [...new Set(Object.values(v))];
  if (values.length !== 1) {
    console.error("version mismatch across single-source copies:");
    for (const [file, ver] of Object.entries(v)) console.error(`  ${file}: ${ver}`);
    console.error("run `node tools/bump-version.mjs` to resync from tauri.conf.json");
    process.exit(1);
  }
  console.log(`version OK: ${values[0]} (4 files agree)`);
  process.exit(0);
}

let target;
if (arg === undefined) {
  target = currentVersions()["tauri.conf.json"];
  console.log(`syncing copies to tauri.conf.json version ${target}`);
} else {
  if (!SEMVER.test(arg)) {
    console.error(`not a valid semver: ${arg}`);
    process.exit(1);
  }
  target = arg;
  const conf = jsonVersion(read(TAURI_CONF), target);
  writeFileSync(TAURI_CONF, conf.text);
  console.log(`tauri.conf.json: ${conf.old} -> ${target}`);
}

const edits = [
  [PACKAGE_JSON, jsonVersion],
  [CARGO_TOML, cargoTomlVersion],
  [CARGO_LOCK, cargoLockVersion],
];
for (const [file, fn] of edits) {
  const res = fn(read(file), target);
  writeFileSync(file, res.text);
  console.log(`${file.split(/[\\/]/).pop()}: ${res.old} -> ${target}`);
}
console.log(`all copies at ${target} (tauri.conf.json is the single source)`);
