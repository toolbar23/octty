import type { OcttyDesktopBridge } from "../shared/desktop-bridge";

declare global {
  interface Window {
    octtyDesktop?: OcttyDesktopBridge;
  }
}

export {};
