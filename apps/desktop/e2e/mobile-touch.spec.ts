/**
 * The Android app is worked by fingers: what a mouse drag, a wheel and the
 * keyboard do on a desktop has to be reachable by touch.
 *
 * Touches are sent through the browser's own input pipeline (the DevTools
 * `Input.dispatchTouchEvent`), so the page receives real pointer events of
 * type `touch`, with the browser deciding what is a scroll and what is the
 * page's, as it does on a device.
 */

import type { Page } from "@playwright/test";

import { androidCommands, nodeId, ok, projectWith, selecting, toneTrack } from "./backend";
import { expect, test, type App } from "./fixtures";
import { laneSurface } from "./timeline";

type Point = { x: number; y: number; id?: number };

const sessions = new WeakMap<Page, Promise<import("@playwright/test").CDPSession>>();

/**
 * Send one touch event with these fingers down. One DevTools session for
 * the page: the browser keeps a gesture's state per session, and a move
 * sent on a new one has no touch to belong to.
 */
async function touch(page: Page, type: "touchStart" | "touchMove" | "touchEnd", points: Point[]) {
  let session = sessions.get(page);
  if (!session) {
    session = page.context().newCDPSession(page);
    sessions.set(page, session);
  }
  await (await session).send("Input.dispatchTouchEvent", {
    type,
    touchPoints: points.map((p, i) => ({ x: p.x, y: p.y, id: p.id ?? i })),
  });
}

/** One finger, dragged in a straight line. */
async function drag(page: Page, from: Point, to: Point, steps = 8) {
  await touch(page, "touchStart", [from]);
  for (let i = 1; i <= steps; i++) {
    const t = i / steps;
    await touch(page, "touchMove", [
      { x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t },
    ]);
  }
  await touch(page, "touchEnd", []);
}

/** The last selection the app told the backend about. */
async function lastSelection(app: App) {
  const sent = (await app.requestsFor("set_selection_context")) as {
    range: { start_sec: number; end_sec: number } | null;
  }[];
  return sent.length ? sent[sent.length - 1].range : undefined;
}

/** A project holding the three-second tone, with the timeline on screen. */
async function openTimeline(app: App, project: string) {
  await app.boot({
    ...projectWith([toneTrack()], nodeId(3)),
    ...androidCommands(),
    ...selecting(),
  });
  if (project === "phone") await app.page.getByTestId("mobile-tab-timeline").tap();
  await expect(app.page.getByTestId("ruler")).toContainText("0:03");
  const box = await laneSurface(app.page, 0).boundingBox();
  if (!box) throw new Error("the lane has no box");
  return box;
}

test.describe("the timeline under a finger", () => {
  test("a drag along the lane selects that stretch of time", async ({ app }, info) => {
    const box = await openTimeline(app, info.project.name);
    const y = box.y + box.height / 2;
    await drag(app.page, { x: box.x + box.width * 0.3, y }, { x: box.x + box.width * 0.7, y });

    await expect.poll(() => lastSelection(app), { message: "a selection reaches the backend" }).toBeTruthy();
    const range = (await lastSelection(app))!;
    // 30% to 70% of a three-second file, to within a finger's width.
    expect(range.start_sec).toBeGreaterThan(0.75);
    expect(range.start_sec).toBeLessThan(1.05);
    expect(range.end_sec).toBeGreaterThan(1.95);
    expect(range.end_sec).toBeLessThan(2.25);
    await expect(app.page.getByTestId("timeline-selection-overlay")).toBeVisible();
  });

  test("a tap clears the selection and does not make a new one", async ({ app }, info) => {
    const box = await openTimeline(app, info.project.name);
    const y = box.y + box.height / 2;
    await drag(app.page, { x: box.x + box.width * 0.2, y }, { x: box.x + box.width * 0.6, y });
    await expect.poll(() => lastSelection(app)).toBeTruthy();

    await app.page.touchscreen.tap(box.x + box.width * 0.8, y);
    await expect.poll(() => lastSelection(app), { message: "the selection is cleared" }).toBeNull();
    await expect(app.page.getByTestId("timeline-selection-overlay")).toHaveCount(0);
  });

  test("two fingers spreading zoom in", async ({ app }, info) => {
    const box = await openTimeline(app, info.project.name);
    const page = app.page;
    const y = box.y + box.height / 2;
    const mid = box.x + box.width / 2;
    // At fit, the ruler's last label is the end of the file.
    await expect(page.getByTestId("ruler")).toContainText("0:02.5");

    await touch(page, "touchStart", [{ x: mid - 20, y, id: 0 }]);
    await touch(page, "touchStart", [
      { x: mid - 20, y, id: 0 },
      { x: mid + 20, y, id: 1 },
    ]);
    for (const spread of [40, 60, 80, 100]) {
      await touch(page, "touchMove", [
        { x: mid - spread, y, id: 0 },
        { x: mid + spread, y, id: 1 },
      ]);
    }
    await touch(page, "touchEnd", []);

    // Five times the density: the pane now shows well under a second, so
    // the ruler no longer reaches the two-and-a-half second mark.
    await expect(page.getByTestId("ruler")).not.toContainText("0:02.5");
    // A pinch is not a selection.
    expect(await lastSelection(app)).toBeFalsy();
  });

  test("the toolbar is all reachable: it scrolls, the page does not", async ({ app }, info) => {
    await openTimeline(app, info.project.name);
    const page = app.page;
    const loop = page.getByTestId("loop-btn");
    await loop.scrollIntoViewIfNeeded();
    await expect(loop).toBeInViewport();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
});

test.describe("the transcript under a finger", () => {
  // `get_transcript` for a track that was transcribed: `TranscriptOut`
  // with the words and the track they came from (`commands.rs`).
  const words = ["one", "two", "three", "four", "five"].map((text, i) => ({
    text,
    start_sec: i * 0.5,
    end_sec: i * 0.5 + 0.4,
    confidence: 0.9,
  }));

  test("tap a word, tap another, and Cut removes the words between", async ({ app }, info) => {
    const track = toneTrack();
    const after = nodeId(9);
    await app.boot({
      ...projectWith([track], nodeId(3)),
      ...androidCommands(),
      ...selecting(),
      get_transcript: ok({ words, source: { kind: "track", track: 0 } }),
      // `cut_transcript_words` answers the new head.
      cut_transcript_words: ok(after),
    });
    const page = app.page;
    if (info.project.name === "phone") await page.getByTestId("mobile-tab-transcript").tap();
    else await page.getByTestId("tab-transcript").tap();

    const word = page.getByTestId("transcript-word");
    await expect(word).toHaveCount(5);
    // Nothing selected, no bar.
    await expect(page.getByTestId("transcript-actions")).toHaveCount(0);

    await word.nth(1).tap();
    await word.nth(3).tap();
    await expect(word.nth(1)).toHaveAttribute("data-selected", "true");
    await expect(word.nth(2)).toHaveAttribute("data-selected", "true");
    await expect(word.nth(3)).toHaveAttribute("data-selected", "true");
    await expect(word.nth(4)).toHaveAttribute("data-selected", "false");

    await expect(page.getByTestId("transcript-cut")).toContainText("3 words");
    await page.getByTestId("transcript-cut").tap();
    await expect
      .poll(() => app.requestsFor("cut_transcript_words"))
      .toEqual([{ track: 0, fromWord: 1, toWord: 4 }]);
  });
});

test.describe("History under a finger", () => {
  test("press and hold on a step opens its menu", async ({ app }, info) => {
    const A = nodeId(3);
    const B = nodeId(4);
    await app.boot({
      ...projectWith([toneTrack()], B),
      ...androidCommands(),
      // `get_graph`: a load, and one edit after it, with the head on the edit.
      get_graph: ok({
        nodes: [
          { id: A, parent: null, label: "load", tool: "load", created_at: "2026-09-25T00:00:00Z" },
          { id: B, parent: A, label: "gain", tool: "gain", created_at: "2026-09-25T00:01:00Z" },
        ],
        head: B,
      }),
    });
    const page = app.page;
    if (info.project.name === "phone") await page.getByTestId("mobile-tab-graph").tap();
    else await page.getByTestId("tab-graph").tap();

    const bubble = page.getByTestId("graph-bubble").first();
    await expect(bubble).toBeVisible();
    const box = (await bubble.boundingBox())!;
    const at = { x: box.x + box.width / 2, y: box.y + box.height / 2 };

    // A right-click has no finger. Holding still is the touch version, and
    // the browser reports it as the same event the desktop's menu answers.
    await touch(page, "touchStart", [at]);
    await page.waitForTimeout(900);
    await touch(page, "touchEnd", []);

    const menu = page.getByTestId("graph-context-menu");
    await expect(menu).toBeVisible();
    await expect(menu).toContainText("Set as head");
  });
});
