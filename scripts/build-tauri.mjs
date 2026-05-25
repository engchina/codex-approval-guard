import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const proxyScript = path.join(repoRoot, "scripts", "with-crates-proxy.mjs");
const bundleTarget = platformBundleTarget(process.platform);
const cargoTargetDir =
  process.env.CARGO_TARGET_DIR ?? path.join(repoRoot, "src-tauri", "target", "package");

if (!bundleTarget) {
  console.error("Linux builds are not supported by codex-approval-guard.");
  process.exit(2);
}

const child = spawn(
  process.execPath,
  [proxyScript, "npm", "run", "tauri", "--", "build", "--bundles", bundleTarget],
  {
    cwd: repoRoot,
    env: {
      ...process.env,
      CARGO_TARGET_DIR: cargoTargetDir,
    },
    stdio: "inherit",
  },
);

child.on("error", (error) => {
  console.error(error);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});

function platformBundleTarget(platform) {
  if (platform === "win32") {
    return "nsis";
  }
  if (platform === "darwin") {
    return "app,dmg";
  }
  return null;
}
