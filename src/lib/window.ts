import { create } from "zustand";
import { appWindow } from "./preview";

const MAC = navigator.userAgent.includes("Mac");

/**
 * On macOS the window's own buttons sit in the top bar (`titleBarStyle: Overlay` in tauri.conf.json), so the bar starts
 * after them; in full screen macOS hides them and the bar starts at the edge again. Elsewhere the system's title bar
 * stays above the app.
 */
export const useWindowButtons = create(() => ({ inBar: MAC }));
if (MAC) void appWindow.onResized(async () => useWindowButtons.setState({ inBar: !(await appWindow.isFullscreen()) }));

/** The window names the open project, as the window switcher and the taskbar show it; the home screen names the app. */
export function titleWindow(project: string | null) {
  void appWindow.setTitle(project ? `${project} — Nuzky` : "Nuzky");
}
