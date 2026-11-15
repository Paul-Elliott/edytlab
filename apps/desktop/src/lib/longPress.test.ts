import type { MouseEvent, PointerEvent } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { HOLD_MS, longPress, type PressPoint } from "./longPress";

const touch = (x: number, y: number) =>
  ({ pointerType: "touch", clientX: x, clientY: y }) as PointerEvent;
const rightClick = (x: number, y: number) => {
  const event = { clientX: x, clientY: y, preventDefault: vi.fn(), stopPropagation: vi.fn() };
  return event as unknown as MouseEvent & { preventDefault: ReturnType<typeof vi.fn> };
};

describe("longPress", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("calls the handler once for a finger held still, where it rests", () => {
    const seen: PressPoint[] = [];
    const press = longPress((at) => seen.push(at));
    press.onPointerDown(touch(40, 60));
    vi.advanceTimersByTime(HOLD_MS - 1);
    expect(seen).toHaveLength(0);
    vi.advanceTimersByTime(1);
    expect(seen).toHaveLength(1);
    expect(seen[0]).toMatchObject({ clientX: 40, clientY: 60 });

    // The browser's own contextmenu for the same hold is swallowed.
    const echo = rightClick(40, 60);
    press.onContextMenu(echo);
    expect(echo.preventDefault).toHaveBeenCalled();
    expect(seen).toHaveLength(1);
  });

  it("is not a tap, and not a drag", () => {
    const handler = vi.fn();
    const press = longPress(handler);

    press.onPointerDown(touch(0, 0));
    vi.advanceTimersByTime(200);
    press.onPointerUp();
    vi.advanceTimersByTime(HOLD_MS);

    press.onPointerDown(touch(0, 0));
    press.onPointerMove(touch(30, 0));
    vi.advanceTimersByTime(HOLD_MS);

    // A finger that only trembles is still resting.
    press.onPointerDown(touch(0, 0));
    press.onPointerMove(touch(3, 4));
    vi.advanceTimersByTime(HOLD_MS);

    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("leaves a mouse alone: only its right-click asks", () => {
    const handler = vi.fn();
    const press = longPress(handler);
    press.onPointerDown({ pointerType: "mouse", clientX: 1, clientY: 1 } as PointerEvent);
    vi.advanceTimersByTime(HOLD_MS * 2);
    expect(handler).not.toHaveBeenCalled();

    vi.setSystemTime(Date.now() + 5_000);
    const click = rightClick(7, 9);
    press.onContextMenu(click);
    expect(handler).toHaveBeenCalledWith(click);
  });
});
