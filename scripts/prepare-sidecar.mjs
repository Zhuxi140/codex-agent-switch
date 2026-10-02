import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
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
const development = process.argv.includes("--dev");
let target = join(binaries, `cas-helper-${triple}${extension}`);
if (development) {
  // 使用不可变路径，不覆盖仍被 Codex MCP 占用的旧 helper。
  const hash = createHash("sha256").update(readFileSync(source)).digest("hex");
  target = join(binaries, "dev", hash, `cas-helper${extension}`);
  mkdirSync(dirname(target), { recursive: true });
  if (!existsSync(target)) copyFileSync(source, target);
  const marker = join(binaries, "dev-helper.sha256");
  if (!existsSync(marker) || readFileSync(marker, "utf8").trim() !== hash) {
    writeFileSync(marker, `${hash}\n`);
  }
} else {
  mkdirSync(binaries, { recursive: true });
  copyFileSync(source, target);
}
console.log(`Prepared sidecar: ${target}`);
