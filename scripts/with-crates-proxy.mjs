import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const cargoHome = path.join(repoRoot, "cargo-home");
const parsed = parseArgs(process.argv.slice(2));
const preferredPort = Number(process.env.CRATES_PROXY_PORT ?? 38281);

if (parsed.command.length === 0) {
  console.error("Usage: node scripts/with-crates-proxy.mjs [--cwd <path>] <command> [...args]");
  process.exit(2);
}

const server = createServer(async (request, response) => {
  try {
    if (!request.url) {
      response.writeHead(400);
      response.end("missing url");
      return;
    }

    const url = new URL(request.url, "http://127.0.0.1");
    const upstream = upstreamUrl(url);
    const upstreamResponse = await fetch(upstream, {
      headers: {
        "user-agent": "codex-approval-guard-cargo-proxy",
        accept: request.headers.accept ?? "*/*",
      },
    });

    if (url.pathname === "/config.json") {
      const config = await upstreamResponse.json();
      config.dl = `http://127.0.0.1:${server.address().port}/crates`;
      config.api = "https://crates.io";
      writeJson(response, upstreamResponse.status, config);
      return;
    }

    response.writeHead(upstreamResponse.status, {
      "content-type": upstreamResponse.headers.get("content-type") ?? "application/octet-stream",
    });
    response.end(Buffer.from(await upstreamResponse.arrayBuffer()));
  } catch (error) {
    response.writeHead(502, { "content-type": "text/plain; charset=utf-8" });
    response.end(String(error));
  }
});

await listenWithFallback(server, preferredPort);
const activePort = server.address().port;

await mkdir(cargoHome, { recursive: true });
await writeFile(
  path.join(cargoHome, "config.toml"),
  [
    "[source.crates-io]",
    'replace-with = "node-proxy"',
    "",
    "[source.node-proxy]",
    `registry = "sparse+http://127.0.0.1:${activePort}/"`,
    "",
    "[registries.crates-io]",
    'protocol = "sparse"',
    "",
  ].join("\n"),
);

const childCommand = resolveCommand(parsed.command);
const child = spawn(childCommand.file, childCommand.args, {
  cwd: parsed.cwd,
  env: {
    ...process.env,
    CARGO_HOME: cargoHome,
    CARGO_REGISTRIES_CRATES_IO_PROTOCOL: "sparse",
  },
  stdio: "inherit",
});

child.on("error", (error) => {
  server.close(() => {
    console.error(error);
    process.exit(1);
  });
});

child.on("exit", (code, signal) => {
  server.close(() => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? 1);
  });
});

function parseArgs(args) {
  let cwd = repoRoot;
  const command = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--cwd") {
      const next = args[index + 1];
      if (!next) {
        throw new Error("--cwd requires a value");
      }
      cwd = path.resolve(repoRoot, next);
      index += 1;
      continue;
    }
    command.push(...args.slice(index));
    break;
  }

  return { cwd, command };
}

function upstreamUrl(url) {
  if (url.pathname.startsWith("/crates/")) {
    return `https://static.crates.io${url.pathname}${url.search}`;
  }
  return `https://index.crates.io${url.pathname}${url.search}`;
}

async function listenWithFallback(server, port) {
  try {
    await listen(server, port);
  } catch (error) {
    if (process.env.CRATES_PROXY_PORT || error?.code !== "EADDRINUSE") {
      throw error;
    }
    await listen(server, 0);
  }
}

function listen(server, port) {
  return new Promise((resolve, reject) => {
    const onError = (error) => {
      server.off("listening", onListening);
      reject(error);
    };
    const onListening = () => {
      server.off("error", onError);
      resolve();
    };
    server.once("error", onError);
    server.once("listening", onListening);
    server.listen(port, "127.0.0.1");
  });
}

function resolveCommand(command) {
  const [file, ...args] = command;
  if (process.platform !== "win32") {
    return { file, args };
  }

  if (file === "npm") {
    const npmCli = npmCliPath();
    if (npmCli) {
      return { file: process.execPath, args: [npmCli, ...args] };
    }
    return { file: "cmd.exe", args: ["/d", "/s", "/c", "npm", ...args] };
  }
  if (file === "cargo") {
    return { file: "cargo.exe", args };
  }
  return { file, args };
}

function npmCliPath() {
  if (process.env.npm_execpath && existsSync(process.env.npm_execpath)) {
    return process.env.npm_execpath;
  }

  const besideNode = path.join(path.dirname(process.execPath), "node_modules", "npm", "bin", "npm-cli.js");
  if (existsSync(besideNode)) {
    return besideNode;
  }

  return null;
}

function writeJson(response, status, value) {
  response.writeHead(status, { "content-type": "application/json; charset=utf-8" });
  response.end(JSON.stringify(value));
}
