/**
 * longPress — the touch version of a right-click.
 *
 * The desktop opens its menus, and removes a marker or a label, on
 * `contextmenu`. A finger has no right button. Android's WebView raises
 * `contextmenu` for a press that is held, but whether it does depends on the
 * WebView and on what is under the finger, and a menu that sometimes opens
 * is worse than one that never does. So the app times the press itself:
 * a finger held still for {@link HOLD_MS} is the same request.
 *
 * `longPress(handler)` gives the props for one element. They replace its
 * `onContextMenu`: a real right-click still calls the handler, a held touch
 * calls it once, and the `contextmenu` the browser may raise for that same
 * hold is swallowed, so the handler never runs twice.
 */

import type { MouseEvent, PointerEvent } from "react";

/** How long a finger must rest. The platform's own long press is about this. */
export const HOLD_MS = 550;
/** How far it may wander and still be resting, in pixels. */
export const SLOP_PX = 10;

/** What a handler is given: where, and the two things handlers call. */
export interface PressPoint {
  clientX: number;
  clientY: number;
  preventDefault(): void;
  stopPropagation(): void;
}

// One finger does one thing at a time, so one timer serves every element.
let timer: ReturnType<typeof setTimeout> | null = null;
let origin: { x: number; y: number } | null = null;
let firedAt = 0;

function cancel() {
  if (timer !== null) clearTimeout(timer);
  timer = null;
  origin = null;
}

/**
 * Lifting the finger after a hold must not also be a tap. The browser would
 * follow it with the mouse press and click a tap makes, which would close
 * the menu the hold has just opened (it closes on a press outside itself)
 * and act on whatever is under the finger.
 *
 * Cancelling the `touchend` is what stops that. It is caught on the window
 * in the capture phase, because some surfaces (the history graph's nodes)
 * stop the event on its way up and a handler on the element or on React's
 * root would never see it.
 */
function swallowTheTapThatFollows() {
  if (typeof window === "undefined") return;
  const swallow = (e: Event) => {
    if (e.cancelable) e.preventDefault();
  };
  const options = { capture: true, once: true, passive: false } as const;
  window.addEventListener("touchend", swallow, options);
  // A hold that ends some other way (the touch is cancelled) leaves the
  // listener behind; take it away before it could eat a later tap.
  setTimeout(() => window.removeEventListener("touchend", swallow, options), 10_000);
}

export function longPress(handler: (at: PressPoint) => void) {
  return {
    onPointerDown(e: PointerEvent) {
      if (e.pointerType !== "touch") return;
      cancel();
      const at = { clientX: e.clientX, clientY: e.clientY };
      origin = { x: e.clientX, y: e.clientY };
      timer = setTimeout(() => {
        timer = null;
        origin = null;
        firedAt = Date.now();
        swallowTheTapThatFollows();
        handler({ ...at, preventDefault() {}, stopPropagation() {} });
      }, HOLD_MS);
    },
    onPointerMove(e: PointerEvent) {
      if (origin && Math.hypot(e.clientX - origin.x, e.clientY - origin.y) > SLOP_PX) cancel();
    },
    onPointerUp: cancel,
    onPointerCancel: cancel,
    onPointerLeave: cancel,
    onContextMenu(e: MouseEvent) {
      // The browser's own report of the hold this module has just answered.
      if (Date.now() - firedAt < 1000) {
        e.preventDefault();
        return;
      }
      handler(e);
    },
  };
}
