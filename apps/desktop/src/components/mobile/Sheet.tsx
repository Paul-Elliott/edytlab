/**
 * Sheet — the Android app's dialog surface.
 *
 * On a phone it rises from the bottom edge, full width, where a thumb
 * reaches; on a tablet it is a card in the middle, as the desktop's dialogs
 * are. Focus, Escape and the inert background come from `useDialog`, so the
 * Android back button (which closes the top dialog, see `useBackButton`)
 * and the desktop's Escape key end up in the same place.
 */

import type { ReactNode } from "react";

import { useDialog } from "../../hooks/useDialog";
import { usePresence } from "../../hooks/usePresence";

export interface SheetProps {
  open: boolean;
  onClose: () => void;
  title: string;
  testId?: string;
  children: ReactNode;
}

export function Sheet({ open, onClose, title, testId, children }: SheetProps) {
  const { mounted, leaving, onAnimationEnd } = usePresence(open);
  const { dialogProps, headingId } = useDialog(open && mounted, onClose);
  if (!mounted) return null;
  return (
    <div
      className={`${leaving ? "backdrop-out" : "backdrop-in"} fixed inset-0 z-50 flex items-end justify-center bg-black/60 sm:items-center`}
      inert={!open}
      aria-hidden={!open ? true : undefined}
      role="presentation"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onAnimationEnd={onAnimationEnd}
    >
      <div
        {...dialogProps}
        data-testid={testId}
        className={`${leaving ? "overlay-out" : "overlay-in"} sheet-card flex w-full flex-col border border-[var(--border-strong)] bg-[var(--surface-elev)] shadow-(--shadow-dialog) sm:max-w-lg`}
      >
        <div className="flex shrink-0 items-center gap-3 border-b border-[var(--border)] px-4 py-3">
          <h2
            id={headingId}
            className="min-w-0 flex-1 truncate text-sm font-semibold uppercase tracking-wide text-[var(--text)]"
          >
            {title}
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            data-testid={testId ? `${testId}-close` : undefined}
            className="btn-tool touch-target inline-flex items-center justify-center rounded-md"
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" aria-hidden="true">
              <path d="M3 3l8 8M11 3l-8 8" />
            </svg>
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">{children}</div>
      </div>
    </div>
  );
}
