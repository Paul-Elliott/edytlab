/**
 * MobileTabBar — the phone's way between the four panes.
 *
 * The desktop shows the chat beside the timeline. A phone has room for one
 * pane, so the four are tabs along the bottom edge: Chat first, because the
 * app is driven by talking to it, then the three views the desktop's header
 * tabs switch between.
 */

import type { ReactNode } from "react";

import type { LeftView } from "../../lib/views";

/** The pane a phone is showing. The tablet and the desktop show two. */
export type PhonePane = "chat" | LeftView;

export interface MobileTabBarProps {
  pane: PhonePane;
  onSelect: (pane: PhonePane) => void;
  /** Dot on the Chat tab: the assistant answered while another pane was up. */
  chatUnread?: boolean;
}

const stroke = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.7,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

const TABS: { pane: PhonePane; label: string; icon: ReactNode }[] = [
  {
    pane: "chat",
    label: "Chat",
    icon: <path {...stroke} d="M4 5.5A2.5 2.5 0 0 1 6.5 3h11A2.5 2.5 0 0 1 20 5.5v8a2.5 2.5 0 0 1-2.5 2.5H10l-4.5 4v-4A2.5 2.5 0 0 1 4 13.5v-8Z" />,
  },
  {
    pane: "timeline",
    label: "Timeline",
    icon: <path {...stroke} d="M3 12h2M7 7v10M11 4v16M15 9v6M19 6v12M21 12h0" />,
  },
  {
    pane: "transcript",
    label: "Transcript",
    icon: <path {...stroke} d="M5 6h14M5 10h14M5 14h9M5 18h6" />,
  },
  {
    pane: "graph",
    label: "History",
    icon: (
      <>
        <circle {...stroke} cx="6" cy="6" r="2" />
        <circle {...stroke} cx="6" cy="18" r="2" />
        <circle {...stroke} cx="18" cy="12" r="2" />
        <path {...stroke} d="M6 8v8M8 6h4a4 4 0 0 1 4 4" />
      </>
    ),
  },
];

export function MobileTabBar({ pane, onSelect, chatUnread = false }: MobileTabBarProps) {
  return (
    <nav
      data-testid="mobile-tabs"
      aria-label="Panes"
      className="mobile-tabbar flex shrink-0 items-stretch border-t border-[var(--border)] bg-[var(--surface-elev)]"
    >
      {TABS.map((tab) => {
        const active = tab.pane === pane;
        return (
          <button
            key={tab.pane}
            type="button"
            data-testid={`mobile-tab-${tab.pane}`}
            aria-current={active ? "page" : undefined}
            onClick={() => onSelect(tab.pane)}
            className={`relative flex min-h-14 flex-1 flex-col items-center justify-center gap-1 px-1 text-[11px] transition ${
              active ? "text-[var(--accent-text)]" : "text-[var(--text-dim)]"
            }`}
          >
            <svg width="22" height="22" viewBox="0 0 24 24" aria-hidden="true">
              {tab.icon}
            </svg>
            <span>{tab.label}</span>
            {tab.pane === "chat" && chatUnread && !active ? (
              <span
                data-testid="mobile-tab-chat-unread"
                aria-label="New reply"
                className="absolute right-[calc(50%-18px)] top-2 h-2 w-2 rounded-full bg-[var(--accent)]"
              />
            ) : null}
          </button>
        );
      })}
    </nav>
  );
}
