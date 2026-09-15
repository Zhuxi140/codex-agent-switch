import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tauriRoot = join(root, "src-tauri");
const manifest = join(tauriRoot, "Cargo.toml");
const triple = execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
const extension = process.platform === "win32" ? ".exe" : "";

execFileSync(
  "cargo",
  ["build", "--release", "--manifest-path", manifest, "-p", "cas-helper"],
  {
    cwd: root,
    env: {
      ...process.env,
      TAURI_CONFIG: JSON.stringify({ bundle: { externalBin: [] } }),
    },
    stdio: "inherit",
  },
);

const source = join(tauriRoot, "target", "release", `cas-helper${extension}`);
const binaries = join(tauriRoot, "binaries");
const target = join(binaries, `cas-helper-${triple}${extension}`);
mkdirSync(binaries, { recursive: true });
copyFileSync(source, target);
console.log(`Prepared sidecar: ${target}`);
