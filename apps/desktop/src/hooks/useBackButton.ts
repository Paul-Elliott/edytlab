/**
 * The Android back button (and the back swipe that stands in for it).
 *
 * Android gives the press to the app only if the app asks for it; once it
 * does, nothing happens by itself any more, so what "back" means is decided
 * here in one place:
 *
 * 1. a dialog or sheet is open: close the top one (what Escape does on a
 *    desktop);
 * 2. otherwise, whatever the caller says (`onBack` answers whether it used
 *    the press: a phone goes from the timeline back to the chat);
 * 3. otherwise leave the app, without ending it: the task goes behind the
 *    launcher as any Android app's does, and a turn that is running goes on.
 *
 * The press reaches the page in two steps, a window event in between, so
 * that a test can make one with `window.dispatchEvent(new Event(BACK_EVENT))`
 * and the policy above is exercised in a browser, where there is no button.
 */

import { useEffect, useRef } from "react";
import { onBackButtonPress } from "@tauri-apps/api/app";

import { leaveApp } from "../lib/tauri-bridge";
import { hasOpenDialog } from "./useDialog";

/** Dispatched on `window` for each press of the Android back button. */
export const BACK_EVENT = "edytlab:back";

export function useBackButton(enabled: boolean, onBack: () => boolean): void {
  const handler = useRef(onBack);
  useEffect(() => {
    handler.current = onBack;
  });

  // Android's presses, turned into the window event.
  useEffect(() => {
    if (!enabled) return undefined;
    let stopped = false;
    let stop: (() => void) | null = null;
    onBackButtonPress(() => {
      window.dispatchEvent(new Event(BACK_EVENT));
    })
      .then((listener) => {
        const unregister = () => void listener.unregister().catch(() => undefined);
        if (stopped) unregister();
        else stop = unregister;
      })
      // Not on Android, or an older shell: the system keeps its own "back".
      .catch(() => undefined);
    return () => {
      stopped = true;
      stop?.();
    };
  }, [enabled]);

  // What a press does.
  useEffect(() => {
    if (!enabled) return undefined;
    const onPress = () => {
      if (hasOpenDialog()) {
        // The dialogs close on Escape (`useDialog`), top one first.
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        return;
      }
      if (handler.current()) return;
      void leaveApp().catch(() => undefined);
    };
    window.addEventListener(BACK_EVENT, onPress);
    return () => window.removeEventListener(BACK_EVENT, onPress);
  }, [enabled]);
}
