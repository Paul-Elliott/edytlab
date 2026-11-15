/**
 * platform — where the window is running, and how much room it has.
 *
 * The Android app is the same frontend in the system WebView. Two things
 * differ there and every difference in the window hangs off them:
 *
 * - **Android or not.** No folder paths, no native menu, no mouse: projects
 *   come from a list, audio from the system picker, exports leave through
 *   the share sheet, and what was a right-click is a long press.
 * - **Phone or tablet.** A tablet has room for the desktop's two panes. A
 *   phone shows one at a time, chosen from a bar at the bottom.
 *
 * Read at run time from the user agent, as `importFormats.ts` already does
 * for the decoder's platform, so one bundle serves every platform and the
 * e2e suite reaches the Android paths by setting a device's user agent.
 */

import { useSyncExternalStore } from "react";

/** What the layout is built for. `desktop` is every non-Android window. */
export type FormFactor = "desktop" | "tablet" | "phone";

/**
 * The narrowest window that gets two panes. 840 px is Material's "expanded"
 * breakpoint: tablets in either orientation and unfolded foldables are over
 * it, phones in portrait are under it. A phone turned sideways is over it in
 * width but has no height for two panes plus a keyboard, hence the height
 * test in {@link formFactorOf}.
 */
export const TABLET_MIN_WIDTH = 840;
const TABLET_MIN_HEIGHT = 520;

/** True in the Android app. False in jsdom, on the desktop and in a browser. */
export function isAndroid(): boolean {
  if (typeof navigator === "undefined") return false;
  return /\bAndroid\b/i.test(navigator.userAgent ?? "");
}

/** The form factor for a window of `width` by `height` CSS pixels. */
export function formFactorOf(android: boolean, width: number, height: number): FormFactor {
  if (!android) return "desktop";
  return width >= TABLET_MIN_WIDTH && height >= TABLET_MIN_HEIGHT ? "tablet" : "phone";
}

function currentFormFactor(): FormFactor {
  if (typeof window === "undefined") return "desktop";
  return formFactorOf(isAndroid(), window.innerWidth, window.innerHeight);
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener("resize", onChange);
  window.addEventListener("orientationchange", onChange);
  return () => {
    window.removeEventListener("resize", onChange);
    window.removeEventListener("orientationchange", onChange);
  };
}

/**
 * The form factor now, updated when the device is turned or a foldable is
 * opened. Always `desktop` outside Android, so a desktop window that is
 * dragged narrow keeps the layout it has always had.
 */
export function useFormFactor(): FormFactor {
  return useSyncExternalStore(subscribe, currentFormFactor, () => "desktop" as const);
}

/**
 * Mark the document for the stylesheet and lock the page's own zoom. Called
 * once, before the first render (`main.tsx`), so the first frame already has
 * the Android rules. Does nothing anywhere else.
 *
 * The zoom lock is for the timeline: two fingers on it change the
 * timeline's zoom, which the page zooming under them at the same time would
 * make unusable. It is set here and not in `index.html` so a desktop
 * window keeps the browser's zoom.
 */
export function applyPlatformMark(): void {
  if (typeof document === "undefined" || !isAndroid()) return;
  document.documentElement.dataset.platform = "android";
  const viewport = document.querySelector('meta[name="viewport"]');
  if (viewport) {
    viewport.setAttribute(
      "content",
      "width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no, viewport-fit=cover",
    );
  }
}
