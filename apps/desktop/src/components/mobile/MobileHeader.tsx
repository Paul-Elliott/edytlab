/**
 * MobileHeader — the Android app's header.
 *
 * The desktop header is one row of a wordmark, three tabs and eight
 * buttons, about a thousand pixels of it. This one keeps what is used all
 * the time within reach (import, undo, redo, record) and puts the rest
 * behind one menu button. On a tablet it also carries the view tabs; a
 * phone has them in the bar at the bottom (`MobileTabBar`).
 *
 * Presentational, like `AppHeader`: every action is a prop.
 */

import type { LeftView } from "../../lib/views";
import { LogoMark } from "../LogoMark";

export interface MobileHeaderProps {
  /** Tablet only: the view tabs. Omitted on a phone. */
  leftView?: LeftView;
  onSelectView?: (view: LeftView) => void;
  onImport: () => void;
  onUndo: () => void;
  onRedo: () => void;
  isRecording: boolean;
  onRecord: () => void;
  /** Recording is refused while something else holds the microphone. */
  recordDisabled?: boolean;
  onMenu: () => void;
}

const VIEWS: { view: LeftView; label: string }[] = [
  { view: "timeline", label: "Timeline" },
  { view: "transcript", label: "Transcript" },
  { view: "graph", label: "History" },
];

const iconButton =
  "btn-tool touch-target inline-flex shrink-0 items-center justify-center rounded-md transition disabled:opacity-40";

const stroke = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.7,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

export function MobileHeader({
  leftView,
  onSelectView,
  onImport,
  onUndo,
  onRedo,
  isRecording,
  onRecord,
  recordDisabled = false,
  onMenu,
}: MobileHeaderProps) {
  return (
    <header
      data-testid="mobile-header"
      className="mobile-header relative z-10 flex shrink-0 items-center gap-1.5 border-b border-[var(--border)] bg-[var(--surface-elev)] px-2 py-1.5"
    >
      <span className="flex shrink-0 items-center px-1" aria-label="edytlab">
        <LogoMark size={26} />
      </span>

      {leftView && onSelectView ? (
        <div className="ml-1 flex items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--surface)] p-0.5">
          {VIEWS.map(({ view, label }) => (
            <button
              key={view}
              type="button"
              data-testid={`tab-${view}`}
              aria-pressed={leftView === view}
              onClick={() => onSelectView(view)}
              className={`min-h-9 rounded px-3 text-xs transition ${
                leftView === view
                  ? "bg-[var(--accent-soft)] text-[var(--accent-text)]"
                  : "text-[var(--text-dim)]"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      ) : null}

      <div className="ml-auto flex items-center gap-1">
        <button type="button" data-testid="mobile-undo" aria-label="Undo" onClick={onUndo} className={iconButton}>
          <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
            <path {...stroke} d="M9 7 5 11l4 4M5 11h9a5 5 0 0 1 0 10h-3" />
          </svg>
        </button>
        <button type="button" data-testid="mobile-redo" aria-label="Redo" onClick={onRedo} className={iconButton}>
          <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
            <path {...stroke} d="m15 7 4 4-4 4M19 11h-9a5 5 0 0 0 0 10h3" />
          </svg>
        </button>
        <button
          type="button"
          data-testid="record-btn"
          aria-label={isRecording ? "Stop recording" : "Record"}
          onClick={onRecord}
          disabled={recordDisabled && !isRecording}
          className={`${iconButton} ${isRecording ? "animate-pulse !border-[var(--danger)] !bg-[var(--danger)] !text-[var(--accent-fg)]" : ""}`}
        >
          <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
            {isRecording ? (
              <rect x="7" y="7" width="10" height="10" rx="1.5" fill="currentColor" />
            ) : (
              <circle cx="12" cy="12" r="6" fill="var(--danger)" />
            )}
          </svg>
        </button>
        <button
          type="button"
          data-testid="open-audio-button"
          onClick={onImport}
          className="btn-secondary inline-flex min-h-11 shrink-0 items-center gap-1.5 rounded-md px-3 text-sm"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
            <path {...stroke} d="M12 4v11m0 0-4-4m4 4 4-4M5 19h14" />
          </svg>
          Import
        </button>
        <button type="button" data-testid="mobile-menu-button" aria-label="Menu" onClick={onMenu} className={iconButton}>
          <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
            <path {...stroke} d="M5 7h14M5 12h14M5 17h14" />
          </svg>
        </button>
      </div>
      <span aria-hidden="true" className="rule-accent -bottom-px" />
    </header>
  );
}
