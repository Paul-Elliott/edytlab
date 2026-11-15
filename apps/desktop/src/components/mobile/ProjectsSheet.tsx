/**
 * ProjectsSheet — the Android app's New, Open and Duplicate project.
 *
 * On a desktop a project is a folder the person picks. Android has no
 * folder picker and no folders to show, so projects live in the app's own
 * storage and are chosen here by name: start one, open one, copy the open
 * one under a new name, or delete one to get its space back.
 *
 * The sheet reads and changes the list itself. Opening and duplicating go
 * back to the application, which owns the order of session changes.
 */

import { useCallback, useEffect, useState } from "react";

import { confirmAction } from "../../lib/confirmAction";
import {
  createManagedProject,
  deleteManagedProject,
  listManagedProjects,
  type RecentProject,
} from "../../lib/tauri-bridge";
import { relativeTime } from "../RecentProjects";
import { Sheet } from "./Sheet";

export interface ProjectsSheetProps {
  open: boolean;
  onClose: () => void;
  /** The folder of the project that is open, to mark its row. */
  currentPath: string | null;
  /** Open the project in `path` (a new one is an empty folder). */
  onOpenProject: (path: string) => Promise<void> | void;
  /** Copy the open project into the empty folder `path` and continue there. */
  onDuplicateInto: (path: string) => Promise<void> | void;
}

export function ProjectsSheet({
  open,
  onClose,
  currentPath,
  onOpenProject,
  onDuplicateInto,
}: ProjectsSheetProps) {
  const [projects, setProjects] = useState<RecentProject[]>([]);
  const [newName, setNewName] = useState("");
  const [copyName, setCopyName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return undefined;
    let cancelled = false;
    setError(null);
    listManagedProjects()
      .then((list) => {
        if (!cancelled) setProjects(list);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  /** Run one action at a time and show what went wrong in the sheet. */
  const run = useCallback(async (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const create = () =>
    run(async () => {
      const path = await createManagedProject(newName);
      setNewName("");
      await onOpenProject(path);
      onClose();
    });

  const duplicate = () =>
    run(async () => {
      const path = await createManagedProject(copyName);
      setCopyName("");
      await onDuplicateInto(path);
      onClose();
    });

  const openOne = (path: string) =>
    run(async () => {
      await onOpenProject(path);
      onClose();
    });

  const deleteOne = (project: RecentProject) =>
    run(async () => {
      const sure = await confirmAction(
        `Delete "${project.name}" with all its audio and history? This cannot be undone.`,
      );
      if (!sure) return;
      setProjects(await deleteManagedProject(project.path));
    });

  const inManaged = currentPath !== null && projects.some((p) => p.path === currentPath);

  return (
    <Sheet open={open} onClose={onClose} title="Projects" testId="projects-sheet">
      {error ? (
        <p role="alert" data-testid="projects-error" className="mb-3 rounded-md border border-[var(--danger-border)] bg-[var(--danger-soft)] px-3 py-2 text-sm text-[var(--text)]">
          {error}
        </p>
      ) : null}

      <form
        className="mb-4 flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (newName.trim() && !busy) void create();
        }}
      >
        <input
          data-testid="new-project-name"
          aria-label="Name for a new project"
          placeholder="New project name"
          value={newName}
          maxLength={80}
          onChange={(e) => setNewName(e.target.value)}
          className="field min-h-11 min-w-0 flex-1 rounded-md px-3 text-sm"
        />
        <button
          type="submit"
          data-testid="new-project-create"
          disabled={busy || newName.trim() === ""}
          className="btn-primary min-h-11 shrink-0 rounded-md px-4 text-sm disabled:opacity-40"
        >
          Create
        </button>
      </form>

      {projects.length === 0 ? (
        <p data-testid="projects-empty" className="px-1 py-4 text-sm text-[var(--text-dim)]">
          No saved projects yet. Name one above to start.
        </p>
      ) : (
        <ul className="flex flex-col gap-1">
          {projects.map((project) => {
            const current = project.path === currentPath;
            return (
              <li
                key={project.path}
                data-testid="project-row"
                className={`flex items-center gap-2 rounded-lg border px-1 ${
                  current ? "border-[var(--accent-border)] bg-[var(--accent-soft)]" : "border-transparent"
                }`}
              >
                <button
                  type="button"
                  disabled={busy || current}
                  onClick={() => void openOne(project.path)}
                  className="flex min-h-12 min-w-0 flex-1 flex-col items-start justify-center px-2 text-left"
                >
                  <span className="w-full truncate text-sm text-[var(--text)]">{project.name}</span>
                  <span className="font-mono text-[11px] text-[var(--text-faint)]">
                    {current ? "Open now" : relativeTime(project.last_opened_at) || "Not opened yet"}
                  </span>
                </button>
                {current ? null : (
                  <button
                    type="button"
                    aria-label={`Delete ${project.name}`}
                    data-testid="project-delete"
                    disabled={busy}
                    onClick={() => void deleteOne(project)}
                    className="btn-tool touch-target inline-flex shrink-0 items-center justify-center rounded-md"
                  >
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                      <path d="M5 7h14M10 7V5h4v2M7 7l1 12h8l1-12M10 11v5M14 11v5" />
                    </svg>
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}

      <form
        className="mt-5 border-t border-[var(--border)] pt-4"
        onSubmit={(e) => {
          e.preventDefault();
          if (copyName.trim() && !busy) void duplicate();
        }}
      >
        <label htmlFor="duplicate-project-name" className="mb-2 block text-xs text-[var(--text-dim)]">
          {inManaged
            ? "Duplicate the open project under a new name and continue in the copy."
            : "Save the open project under a name, so it is kept in this list."}
        </label>
        <div className="flex gap-2">
          <input
            id="duplicate-project-name"
            data-testid="duplicate-project-name"
            placeholder={inManaged ? "Name for the copy" : "Project name"}
            value={copyName}
            maxLength={80}
            onChange={(e) => setCopyName(e.target.value)}
            className="field min-h-11 min-w-0 flex-1 rounded-md px-3 text-sm"
          />
          <button
            type="submit"
            data-testid="duplicate-project-create"
            disabled={busy || copyName.trim() === ""}
            className="btn-secondary min-h-11 shrink-0 rounded-md px-4 text-sm disabled:opacity-40"
          >
            {inManaged ? "Duplicate" : "Save"}
          </button>
        </div>
      </form>
    </Sheet>
  );
}
