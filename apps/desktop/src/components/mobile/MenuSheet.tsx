/**
 * MenuSheet — everything the Android header has no room to show.
 *
 * The desktop spreads these across its header and its keyboard. Here they
 * are one list of large rows behind the menu button, so nothing in the app
 * depends on a key press or on hovering to find it.
 */

import type { ReactNode } from "react";

import { Sheet } from "./Sheet";

export interface MenuItem {
  id: string;
  label: string;
  /** One line under the label, for a row whose name does not say enough. */
  hint?: string;
  onSelect: () => void;
  disabled?: boolean;
}

export interface MenuSheetProps {
  open: boolean;
  onClose: () => void;
  items: MenuItem[];
  /** Shown under the rows: the app version, say. */
  footer?: ReactNode;
}

export function MenuSheet({ open, onClose, items, footer }: MenuSheetProps) {
  return (
    <Sheet open={open} onClose={onClose} title="Menu" testId="menu-sheet">
      <ul className="flex flex-col">
        {items.map((item) => (
          <li key={item.id}>
            <button
              type="button"
              data-testid={`menu-${item.id}`}
              disabled={item.disabled}
              onClick={() => {
                onClose();
                item.onSelect();
              }}
              className="flex min-h-13 w-full flex-col items-start justify-center rounded-lg px-3 py-2 text-left transition active:bg-[var(--accent-soft)] disabled:opacity-40"
            >
              <span className="text-[15px] text-[var(--text)]">{item.label}</span>
              {item.hint ? (
                <span className="text-xs text-[var(--text-faint)]">{item.hint}</span>
              ) : null}
            </button>
          </li>
        ))}
      </ul>
      {footer ? (
        <div className="mt-3 border-t border-[var(--border)] px-3 pt-3 font-mono text-[11px] text-[var(--text-faint)]">
          {footer}
        </div>
      ) : null}
    </Sheet>
  );
}
