import type { OcttyDesktopBridge } from "../shared/desktop-bridge";

function getDesktopBridge(): OcttyDesktopBridge {
  const bridge = window.octtyDesktop;
  if (!bridge) {
    throw new Error("Octty desktop bridge is unavailable. Start the Electron app.");
  }
  return bridge;
}

export const desktopClient = {
  bridge(): OcttyDesktopBridge {
    return getDesktopBridge();
  },
};
