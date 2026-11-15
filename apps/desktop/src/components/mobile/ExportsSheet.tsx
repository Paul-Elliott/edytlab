/**
 * ExportsSheet — where exported files are on Android, and the way out.
 *
 * A desktop export is written to the path the person chose. Android has no
 * paths to choose, so every export (Export Selection, and whatever the
 * assistant renders or exports) is written to one folder in the app's
 * storage and listed here. From the list a file is shared to another app,
 * saved to a place the system's save dialog offers, or deleted.
 */

import { useCallback, useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";

import { confirmAction } from "../../lib/confirmAction";
import {
  deleteExport,
  listExports,
  saveExportTo,
  shareExport,
  type ExportFile,
} from "../../lib/tauri-bridge";
import { relativeTime } from "../RecentProjects";
import { Sheet } from "./Sheet";

export interface ExportsSheetProps {
  open: boolean;
  onClose: () => void;
  /** Bumped by the application when something may have been exported. */
  refreshKey?: number;
}

/** "3.4 MB", "812 KB". */
export function fileSize(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

export function ExportsSheet({ open, onClose, refreshKey = 0 }: ExportsSheetProps) {
  const [files, setFiles] = useState<ExportFile[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return undefined;
    let cancelled = false;
    setError(null);
    setSaved(null);
    listExports()
      .then((list) => {
        if (!cancelled) setFiles(list);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open, refreshKey]);

  const run = useCallback(async (name: string, work: () => Promise<void>) => {
    setBusy(name);
    setError(null);
    setSaved(null);
    try {
      await work();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }, []);

  const share = (file: ExportFile) => run(file.name, () => shareExport(file.name));

  const saveTo = (file: ExportFile) =>
    run(file.name, async () => {
      // On Android this is the system's "create document" screen, and the
      // answer is a content:// address rather than a path.
      const uri = await save({ title: "Save to", defaultPath: file.name });
      if (!uri) return;
      await saveExportTo(file.name, uri);
      setSaved(file.name);
    });

  const remove = (file: ExportFile) =>
    run(file.name, async () => {
      if (!(await confirmAction(`Delete ${file.name} from the exports?`))) return;
      setFiles(await deleteExport(file.name));
    });

  return (
    <Sheet open={open} onClose={onClose} title="Exports" testId="exports-sheet">
      {error ? (
        <p role="alert" data-testid="exports-error" className="mb-3 rounded-md border border-[var(--danger-border)] bg-[var(--danger-soft)] px-3 py-2 text-sm text-[var(--text)]">
          {error}
        </p>
      ) : null}
      {saved ? (
        <p role="status" data-testid="exports-saved" className="mb-3 rounded-md border border-[var(--border-strong)] px-3 py-2 text-sm text-[var(--text)]">
          Saved {saved}.
        </p>
      ) : null}

      {files.length === 0 ? (
        <p data-testid="exports-empty" className="px-1 py-4 text-sm text-[var(--text-dim)]">
          Nothing exported yet. Ask the assistant to export, or select a range and tap Export
          Selection. The files appear here to share or save.
        </p>
      ) : (
        <ul className="flex flex-col gap-2">
          {files.map((file) => (
            <li
              key={file.name}
              data-testid="export-row"
              className="rounded-lg border border-[var(--border)] bg-[var(--surface)] p-3"
            >
              <div className="flex items-baseline gap-2">
                <span className="min-w-0 flex-1 break-all text-sm text-[var(--text)]">{file.name}</span>
                <span className="shrink-0 font-mono text-[11px] text-[var(--text-faint)]">
                  {fileSize(file.bytes)}
                </span>
              </div>
              <div className="mt-0.5 font-mono text-[11px] text-[var(--text-faint)]">
                {relativeTime(new Date(file.modified_ms).toISOString())}
              </div>
              <div className="mt-2 flex gap-2">
                <button
                  type="button"
                  data-testid="export-share"
                  disabled={busy !== null}
                  onClick={() => void share(file)}
                  className="btn-primary min-h-11 flex-1 rounded-md px-3 text-sm disabled:opacity-40"
                >
                  Share
                </button>
                <button
                  type="button"
                  data-testid="export-save"
                  disabled={busy !== null}
                  onClick={() => void saveTo(file)}
                  className="btn-secondary min-h-11 flex-1 rounded-md px-3 text-sm disabled:opacity-40"
                >
                  Save to…
                </button>
                <button
                  type="button"
                  aria-label={`Delete ${file.name}`}
                  data-testid="export-delete"
                  disabled={busy !== null}
                  onClick={() => void remove(file)}
                  className="btn-tool touch-target inline-flex shrink-0 items-center justify-center rounded-md"
                >
                  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                    <path d="M5 7h14M10 7V5h4v2M7 7l1 12h8l1-12M10 11v5M14 11v5" />
                  </svg>
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </Sheet>
  );
}
