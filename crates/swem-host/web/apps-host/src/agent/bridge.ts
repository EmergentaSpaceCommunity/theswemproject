// The official AppBridge bundle, loaded once and lazily.
//
// A 404 means Apps are disabled on this host; the same semantic operations
// stay available through the ordinary tool path, so the absence is a status
// line, not an error. Both the native-session surface and the independent
// Project surface share this loader; opening either must not require the other.

import { at } from "../base.ts";
import type { OpenedApp } from "./session.ts";

export interface MountedApp {
  teardown(): Promise<void>;
  deliver?(terminal: unknown): Promise<void>;
}

interface AppsBridge {
  mount(options: {
    container: HTMLElement;
    opened: OpenedApp;
    relay: (message: unknown) => Promise<unknown>;
    observation: unknown;
    onStatus: (text: string) => void;
    /// Where the App is told it is, and how it is drawn here.
    platform?: "web" | "desktop" | "mobile";
    displayMode?: "inline" | "fullscreen";
  }): Promise<MountedApp>;
}

declare global {
  interface Window {
    SwemAppsBridge?: AppsBridge;
    SwemLoadAppsBridge?: () => Promise<boolean>;
  }
}

let bridgeReady: Promise<boolean> | null = null;

export function loadBridge(): Promise<boolean> {
  if (bridgeReady) return bridgeReady;
  bridgeReady = new Promise((resolve) => {
    const script = document.createElement("script");
    script.src = at("/apps-bridge.js");
    script.onload = () => resolve(Boolean(window.SwemAppsBridge));
    script.onerror = () => resolve(false);
    document.head.appendChild(script);
  });
  return bridgeReady;
}

window.SwemLoadAppsBridge = loadBridge;
