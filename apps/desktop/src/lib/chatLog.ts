/**
 * chatLog — the chat pane's entries as they are kept with a project (the
 * Android app, where the system stops an app that is out of sight and the
 * chat would otherwise come back empty; `chat_log.rs`).
 *
 * What is saved is the pane's own list. What is read back is not trusted
 * to be that: it is a file, and an older or a later build may have written
 * it. `restoredEntries` keeps what this build can draw and drops the rest.
 */

import type { LogEntry, ToolStatus } from "../hooks/useAgentStream";

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

const TOOL_STATUSES: readonly ToolStatus[] = ["running", "ok", "error", "not_run"];

/**
 * The entries of a saved chat, ready to show again.
 *
 * - An entry this build does not know, or one missing what its kind needs,
 *   is left out. The others keep their order.
 * - Every entry gets a new id. The pane numbers its entries from one each
 *   time the app starts, so a saved id would meet a new entry's.
 * - A tool that was still running when the app stopped did not finish
 *   where anyone could see: it is shown as not run, which is all that is
 *   known.
 */
export function restoredEntries(saved: unknown): LogEntry[] {
  if (!Array.isArray(saved)) return [];
  const out: LogEntry[] = [];
  for (const raw of saved) {
    if (!isRecord(raw)) continue;
    const id = `restored-${out.length}`;
    switch (raw.kind) {
      case "message":
        if ((raw.role === "user" || raw.role === "assistant") && typeof raw.text === "string") {
          out.push({
            kind: "message",
            id,
            role: raw.role,
            text: raw.text,
            ...(Array.isArray(raw.chips) ? { chips: raw.chips as never } : {}),
          });
        }
        break;
      case "tool":
        if (typeof raw.name === "string" && TOOL_STATUSES.includes(raw.status as ToolStatus)) {
          out.push({
            kind: "tool",
            id,
            name: raw.name,
            status: raw.status === "running" ? "not_run" : (raw.status as ToolStatus),
            ...(typeof raw.result === "string" ? { result: raw.result } : {}),
            ...(isRecord(raw.view) ? { view: raw.view as never } : {}),
          });
        }
        break;
      case "node":
        if (typeof raw.nodeId === "string") {
          out.push({ kind: "node", id, nodeId: raw.nodeId as never });
        }
        break;
      case "plan":
        if (Array.isArray(raw.steps)) {
          out.push({ kind: "plan", id, steps: raw.steps as never });
        }
        break;
      case "notice":
        if (typeof raw.text === "string") {
          out.push({ kind: "notice", id, text: raw.text });
        }
        break;
      default:
        break;
    }
  }
  return out;
}
