import { afterEach, describe, expect, it, vi } from "vitest";
import { formFactorOf, isAndroid } from "./platform";

describe("platform", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("is the desktop layout everywhere but Android, at any size", () => {
    expect(formFactorOf(false, 360, 800)).toBe("desktop");
    expect(formFactorOf(false, 1280, 800)).toBe("desktop");
  });

  it("gives a phone one pane and a tablet two", () => {
    expect(formFactorOf(true, 412, 915)).toBe("phone");
    expect(formFactorOf(true, 839, 1200)).toBe("phone");
    expect(formFactorOf(true, 840, 1200)).toBe("tablet");
    expect(formFactorOf(true, 1280, 800)).toBe("tablet");
  });

  it("keeps a phone turned sideways on one pane", () => {
    expect(formFactorOf(true, 915, 412)).toBe("phone");
  });

  it("reads Android from the user agent", () => {
    expect(isAndroid()).toBe(false);
    vi.stubGlobal("navigator", {
      userAgent:
        "Mozilla/5.0 (Linux; Android 15; Pixel 9 Build/AP3A; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/140.0.0.0 Mobile Safari/537.36",
    });
    expect(isAndroid()).toBe(true);
    vi.stubGlobal("navigator", {
      userAgent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/140 Safari/537.36",
    });
    expect(isAndroid()).toBe(false);
  });
});
