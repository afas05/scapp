#!/usr/bin/env node
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const newVersion = process.argv[2];

if (!newVersion || !/^\d+\.\d+\.\d+(?:-[\w.]+)?$/.test(newVersion)) {
  console.error('Usage: node scripts/bump-version.mjs <semver>');
  console.error('Example: node scripts/bump-version.mjs 0.2.0');
  process.exit(1);
}

const targets = [
  { path: resolve(root, 'package.json'), key: 'version' },
  { path: resolve(root, 'src-tauri/tauri.conf.json'), key: 'version' },
];

for (const { path, key } of targets) {
  const raw = readFileSync(path, 'utf8');
  const json = JSON.parse(raw);
  const old = json[key];
  json[key] = newVersion;
  const indent = raw.match(/^(\s+)/m)?.[1].length === 4 ? 4 : 2;
  writeFileSync(path, JSON.stringify(json, null, indent) + '\n');
  console.log(`${path}: ${old} -> ${newVersion}`);
}

const cargoPath = resolve(root, 'src-tauri/Cargo.toml');
const cargo = readFileSync(cargoPath, 'utf8');
const updatedCargo = cargo.replace(
  /^version\s*=\s*"[^"]+"/m,
  `version = "${newVersion}"`,
);
writeFileSync(cargoPath, updatedCargo);
console.log(`${cargoPath}: bumped to ${newVersion}`);

console.log(`\nNext: git add -A && git commit -m "release: v${newVersion}" && git tag v${newVersion}`);
