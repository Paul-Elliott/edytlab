/**
 * Which files the picker offers (#764). M4A/AAC is opened by the operating
 * system's decoder, so it is offered on macOS, Windows and Android and
 * nowhere else.
 * `crates/audio-decoder/tests/supported_extensions.rs` holds these lists equal
 * to the decoder's own.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  BASE_EXTENSIONS,
  importExtensions,
  importFormatNames,
  osDecodesM4a,
} from "../lib/importFormats";

afterEach(() => {
  vi.unstubAllGlobals();
});

const LINUX = { platform: "Linux x86_64", userAgent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15" };
const MAC = { platform: "MacIntel", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X) AppleWebKit/605.1.15" };
const WINDOWS = { platform: "Win32", userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/140" };
// The Android System WebView on a phone and on a tablet (no "Mobile").
const ANDROID_PHONE = {
  platform: "Linux armv8l",
  userAgent:
    "Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/UQ1A.240205.004; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/140.0.0.0 Mobile Safari/537.36",
};
const ANDROID_TABLET = {
  platform: "Linux aarch64",
  userAgent:
    "Mozilla/5.0 (Linux; Android 13; SM-X710 Build/TP1A.220624.014; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/140.0.0.0 Safari/537.36",
};
const IPHONE = {
  platform: "iPhone",
  userAgent:
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148",
};
const IPAD = {
  platform: "iPad",
  userAgent:
    "Mozilla/5.0 (iPad; CPU OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148",
};

describe("importExtensions", () => {
  it("offers the base list on Linux, where there is no system AAC decoder", () => {
    vi.stubGlobal("navigator", LINUX);
    expect(osDecodesM4a()).toBe(false);
    expect(importExtensions()).toEqual(BASE_EXTENSIONS);
    expect(importFormatNames()).not.toContain("m4a");
  });

  it.each([
    ["macOS", MAC],
    ["Windows", WINDOWS],
    ["an Android phone", ANDROID_PHONE],
    ["an Android tablet", ANDROID_TABLET],
  ])("adds m4a on %s", (_name, nav) => {
    vi.stubGlobal("navigator", nav);
    expect(osDecodesM4a()).toBe(true);
    expect(importExtensions()).toEqual([...BASE_EXTENSIONS, "m4a"]);
    expect(importFormatNames()).toContain("m4a");
  });

  it.each([
    ["an iPhone", IPHONE],
    ["an iPad", IPAD],
  ])("offers the base list on %s, whose user agent says Mac and which has no decoder", (_name, nav) => {
    vi.stubGlobal("navigator", nav);
    expect(osDecodesM4a()).toBe(false);
    expect(importExtensions()).toEqual(BASE_EXTENSIONS);
    expect(importFormatNames()).not.toContain("m4a");
  });

  it("offers the base list when there is no navigator at all", () => {
    vi.stubGlobal("navigator", undefined);
    expect(osDecodesM4a()).toBe(false);
    expect(importExtensions()).toEqual(BASE_EXTENSIONS);
  });

  it("never offers a bare .aac file: only M4A is routed to the system decoder", () => {
    for (const nav of [LINUX, MAC, WINDOWS, ANDROID_PHONE, ANDROID_TABLET]) {
      vi.stubGlobal("navigator", nav);
      expect(importExtensions()).not.toContain("aac");
    }
  });
});
