import { cp, copyFile, mkdir, rm } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const currentFile = fileURLToPath(import.meta.url);
const rootDir = resolve(dirname(currentFile), "..");
const outDir = resolve(rootDir, "build", "electron");

await rm(outDir, { recursive: true, force: true });
await mkdir(outDir, { recursive: true });

await Promise.all([
  build({
    entryPoints: [resolve(rootDir, "src", "electron", "main.ts")],
    bundle: true,
    platform: "node",
    format: "esm",
    target: "node20",
    outfile: resolve(outDir, "main.js"),
    external: ["electron"],
    sourcemap: true,
  }),
  build({
    entryPoints: [resolve(rootDir, "src", "electron", "preload.ts")],
    bundle: true,
    platform: "node",
    format: "cjs",
    target: "node20",
    outfile: resolve(outDir, "preload.cjs"),
    external: ["electron"],
    sourcemap: true,
  }),
  build({
    entryPoints: [resolve(rootDir, "src", "mainview", "index.tsx")],
    bundle: true,
    platform: "browser",
    format: "iife",
    target: ["chrome124"],
    outfile: resolve(outDir, "renderer.js"),
    sourcemap: true,
    loader: {
      ".css": "css",
    },
  }),
]);

await Promise.all([
  copyFile(resolve(rootDir, "src", "electron", "index.html"), resolve(outDir, "index.html")),
  copyFile(
    resolve(rootDir, "node_modules", "ghostty-web", "ghostty-vt.wasm"),
    resolve(outDir, "ghostty-vt.wasm"),
  ),
  cp(resolve(rootDir, "src", "pty-host"), resolve(outDir, "runtime", "pty-host"), {
    recursive: true,
  }),
]);
