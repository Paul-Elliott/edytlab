import { describe, expect, it } from "vitest";

import { restoredEntries } from "../lib/chatLog";

describe("restoredEntries", () => {
  it("gives back every kind of entry the pane draws, in order", () => {
    const saved = [
      { kind: "message", id: "e1", role: "user", text: "make it louder" },
      { kind: "plan", id: "e2", steps: [{ tool: "gain", description: "+6 dB" }] },
      { kind: "tool", id: "call_1", name: "gain", status: "ok", result: "+6 dB" },
      { kind: "node", id: "e3", nodeId: "abc123" },
      { kind: "notice", id: "e4", text: "No plan arrived." },
      { kind: "message", id: "e5", role: "assistant", text: "Done.", chips: [{ label: "Undo" }] },
    ];
    const out = restoredEntries(saved);
    expect(out.map((e) => e.kind)).toEqual(["message", "plan", "tool", "node", "notice", "message"]);
    expect(out[0]).toMatchObject({ role: "user", text: "make it louder" });
    expect(out[2]).toMatchObject({ name: "gain", status: "ok", result: "+6 dB" });
    expect(out[3]).toMatchObject({ nodeId: "abc123" });
    expect(out[5]).toMatchObject({ role: "assistant", chips: [{ label: "Undo" }] });
  });

  it("gives every entry a new id, so none meets an id the pane makes after a start", () => {
    const out = restoredEntries([
      { kind: "message", id: "e1", role: "user", text: "a" },
      { kind: "message", id: "e1", role: "assistant", text: "b" },
      { kind: "tool", id: "e2", name: "gain", status: "ok" },
    ]);
    const ids = out.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
    // The pane's own ids are "e1", "e2", ... from one at every start.
    expect(ids.every((id) => !/^e\d+$/.test(id))).toBe(true);
  });

  it("shows a tool that was still running when the app stopped as not run", () => {
    const [tool] = restoredEntries([{ kind: "tool", id: "c", name: "separate_stems", status: "running" }]);
    expect(tool).toMatchObject({ kind: "tool", status: "not_run" });
  });

  it("leaves out what this build cannot draw and keeps the rest", () => {
    const out = restoredEntries([
      null,
      "text",
      { kind: "hologram", id: "x" },
      { kind: "message", id: "e1", role: "system", text: "not a role" },
      { kind: "message", id: "e2", role: "user" },
      { kind: "tool", id: "c", name: "gain", status: "exploded" },
      { kind: "node", id: "n" },
      { kind: "message", id: "e3", role: "user", text: "kept" },
    ]);
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ kind: "message", text: "kept" });
  });

  it("reads anything that is not a list as an empty chat", () => {
    for (const bad of [undefined, null, {}, "[]", 7]) {
      expect(restoredEntries(bad)).toEqual([]);
    }
  });
});
