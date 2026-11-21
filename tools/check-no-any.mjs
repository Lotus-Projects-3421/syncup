#!/usr/bin/env node
// Fails the frontend check when hand-written source contains an explicit `any`.
// Generated IPC bindings are excluded because they must never be edited by hand.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = "src";
const SKIP = new Set(["src/lib/ipc.ts"]);
const ANY_PATTERN = /(?::\s*any\b|<any>|\bas\s+any\b|\bany\s*\[\])/;

function walk(directory) {
  const files = [];
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    const stats = statSync(path);
    if (stats.isDirectory()) {
      files.push(...walk(path));
    } else {
      files.push(path);
    }
  }
  return files;
}

const files = walk(ROOT).filter((file) => {
  if (SKIP.has(file)) return false;
  return (file.endsWith(".ts") && !file.endsWith(".d.ts")) || file.endsWith(".svelte");
});

let failures = 0;

for (const file of files) {
  const lines = readFileSync(file, "utf8").split("\n");
  let inScript = !file.endsWith(".svelte");

  lines.forEach((line, index) => {
    if (file.endsWith(".svelte")) {
      if (/<script\b/.test(line)) inScript = true;
      if (/<\/script>/.test(line)) {
        inScript = false;
        return;
      }
    }

    if (inScript && ANY_PATTERN.test(line)) {
      console.error(`${file}:${index + 1}: explicit \`any\` is not allowed: ${line.trim()}`);
      failures += 1;
    }
  });
}

if (failures > 0) {
  console.error(`\nFound ${failures} explicit \`any\` usage(s).`);
  process.exit(1);
}

console.log(`No explicit \`any\` found in ${files.length} hand-written source file(s).`);
