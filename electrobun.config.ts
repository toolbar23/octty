import type { ElectrobunConfig } from "electrobun";

function readEnvString(...names: string[]): string | undefined {
  for (const name of names) {
    const value = process.env[name]?.trim();
    if (value) {
      return value;
    }
  }
  return undefined;
}

function readEnvBoolean(...names: string[]): boolean | undefined {
  const rawValue = readEnvString(...names);
  if (rawValue === undefined) {
    return undefined;
  }

  const normalized = rawValue.toLowerCase();
  if (["1", "true", "yes", "on"].includes(normalized)) {
    return true;
  }
  if (["0", "false", "no", "off"].includes(normalized)) {
    return false;
  }

  throw new Error(
    `Invalid boolean value "${rawValue}" for ${names.join(" / ")}. Use 1/0 or true/false.`,
  );
}

function readRenderer(...names: string[]): "native" | "cef" | undefined {
  const rawValue = readEnvString(...names);
  if (rawValue === undefined) {
    return undefined;
  }

  if (rawValue === "native" || rawValue === "cef") {
    return rawValue;
  }

  throw new Error(
    `Invalid renderer "${rawValue}" for ${names.join(" / ")}. Use "native" or "cef".`,
  );
}

const linuxChromiumFlags: Record<string, string | boolean> = {};
const defaultLinuxRenderer =
  readRenderer("OCTTY_DEFAULT_RENDERER", "WORKSPACE_ORBIT_DEFAULT_RENDERER") ?? "cef";
const bundleLinuxCef =
  readEnvBoolean("OCTTY_BUNDLE_CEF", "WORKSPACE_ORBIT_BUNDLE_CEF") ?? true;

const cefUserDataDir = readEnvString(
  "OCTTY_CEF_PROFILE_DIR",
  "OCTTY_CEF_USER_DATA_DIR",
  "WORKSPACE_ORBIT_CEF_PROFILE_DIR",
  "WORKSPACE_ORBIT_CEF_USER_DATA_DIR",
);
if (cefUserDataDir) {
  linuxChromiumFlags["user-data-dir"] = cefUserDataDir;
}

const cefRemoteDebuggingPort = readEnvString(
  "OCTTY_CEF_REMOTE_DEBUGGING_PORT",
  "WORKSPACE_ORBIT_CEF_REMOTE_DEBUGGING_PORT",
);
if (cefRemoteDebuggingPort) {
  linuxChromiumFlags["remote-debugging-port"] = cefRemoteDebuggingPort;
}

const cefUseGl = readEnvString(
  "OCTTY_CEF_USE_GL",
  "WORKSPACE_ORBIT_CEF_USE_GL",
);
if (cefUseGl) {
  linuxChromiumFlags["use-gl"] = cefUseGl;
}

const cefOzonePlatform = readEnvString(
  "OCTTY_CEF_OZONE_PLATFORM",
  "WORKSPACE_ORBIT_CEF_OZONE_PLATFORM",
);
if (cefOzonePlatform) {
  linuxChromiumFlags["ozone-platform"] = cefOzonePlatform;
}

const cefDisableGpuCompositing = readEnvBoolean(
  "OCTTY_CEF_DISABLE_GPU_COMPOSITING",
  "WORKSPACE_ORBIT_CEF_DISABLE_GPU_COMPOSITING",
);
if (cefDisableGpuCompositing !== undefined) {
  linuxChromiumFlags["disable-gpu-compositing"] = cefDisableGpuCompositing;
}

export default {
  app: {
    name: "Octty",
    identifier: "dev.pm.octty",
    version: "0.1.0",
  },
  build: {
    bun: {
      entrypoint: "src/bun/index.ts",
    },
    views: {
      mainview: {
        entrypoint: "src/mainview/index.tsx",
      },
    },
    copy: {
      "src/mainview/index.html": "views/mainview/index.html",
      "src/mainview/index.css": "views/mainview/index.css",
      "src/pty-host/index.mjs": "runtime/pty-host/index.mjs",
    },
    linux: {
      bundleCEF: bundleLinuxCef,
      defaultRenderer: defaultLinuxRenderer,
      chromiumFlags:
        Object.keys(linuxChromiumFlags).length > 0 ? linuxChromiumFlags : undefined,
    },
  },
} satisfies ElectrobunConfig;
