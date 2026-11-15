/**
 * The Android app's window: one pane and a tab bar on a phone, two panes on
 * a tablet, and everything the desktop keeps in its header or on its
 * keyboard reachable by a tap.
 *
 * These run in the `phone` and `tablet` projects (`playwright.config.ts`),
 * which give the page an Android user agent, a device's viewport and touch.
 * The build is the one that ships; only the backend is the fake.
 */

import { androidCommands, nodeId, ok, readyToLoad, sessionWith, toneTrack } from "./backend";
import { expect, test } from "./fixtures";


/** A device with a key saved and nothing loaded yet. */
const ready = () => ({ ...readyToLoad(), ...androidCommands() });

test.describe("the Android window", () => {
  test("marks the document and never scrolls sideways", async ({ app }) => {
    await app.boot(ready());
    const page = app.page;
    await expect(page.getByTestId("mobile-header")).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.dataset.platform)).toBe("android");
    // The desktop header is not drawn at all: its row cannot fit.
    await expect(page.getByTestId("left-pane-tabs")).toHaveCount(0);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    );
    expect(overflow, "the page is wider than the screen").toBeLessThanOrEqual(0);
  });

  test("shows one pane with a tab bar on a phone, two panes on a tablet", async ({ app }, info) => {
    await app.boot(ready());
    const page = app.page;
    const chat = page.getByTestId("chat-pane");
    const work = page.getByTestId("work-pane");

    if (info.project.name === "phone") {
      await expect(page.getByTestId("mobile-tabs")).toBeVisible();
      // The chat is what the app starts on.
      await expect(chat).toBeVisible();
      await expect(work).toBeHidden();

      await page.getByTestId("mobile-tab-timeline").tap();
      await expect(work).toBeVisible();
      await expect(chat).toBeHidden();
      await expect(page.getByTestId("empty-state")).toBeVisible();

      await page.getByTestId("mobile-tab-chat").tap();
      await expect(chat).toBeVisible();
    } else {
      await expect(page.getByTestId("mobile-tabs")).toHaveCount(0);
      await expect(chat).toBeVisible();
      await expect(work).toBeVisible();
      // The view tabs are in the header, where the desktop has them.
      await expect(page.getByTestId("tab-transcript")).toBeVisible();
    }
  });

  test("every header control is big enough for a finger", async ({ app }) => {
    await app.boot(ready());
    const sizes = await app.page
      .getByTestId("mobile-header")
      .getByRole("button")
      .evaluateAll((buttons) =>
        buttons.map((b) => {
          const r = b.getBoundingClientRect();
          return { label: b.getAttribute("aria-label") ?? b.textContent, w: r.width, h: r.height };
        }),
      );
    expect(sizes.length).toBeGreaterThanOrEqual(5);
    for (const size of sizes) {
      expect(size.h, `${size.label} is ${size.h}px tall`).toBeGreaterThanOrEqual(36);
      expect(size.w, `${size.label} is ${size.w}px wide`).toBeGreaterThanOrEqual(36);
    }
  });
});

test.describe("importing audio on Android", () => {
  test("hands the picker's addresses to import_audio and shows the track", async ({ app }, info) => {
    const loaded = nodeId(7);
    const uri = "content://com.android.providers.media.documents/document/audio%3A41";
    await app.boot({
      ...ready(),
      // The Android picker answers content:// addresses, not paths.
      "plugin:dialog|open": ok([uri]),
      // `import_audio` answers what `batch_load` does: `BatchLoadResult`.
      import_audio: ok({ tracks_loaded: 1, last_node_id: loaded, failures: [] }),
    });
    const page = app.page;

    await app.become(sessionWith([toneTrack()]));
    await page.getByTestId("open-audio-button").tap();

    // On a phone the import moves to the timeline, where the track is.
    await expect(page.getByTestId("ruler")).toContainText("0:03");
    if (info.project.name === "phone") {
      await expect(page.getByTestId("work-pane")).toBeVisible();
    }
    expect(await app.requestsFor("import_audio")).toEqual([{ uris: [uri] }]);
    // The desktop's path loader is never asked: there is no path.
    expect(await app.requestsFor("batch_load")).toEqual([]);

    const filters = (await app.requestsFor("plugin:dialog|open")) as {
      options: { filters: { extensions: string[] }[]; multiple: boolean };
    }[];
    expect(filters[0].options.multiple).toBe(true);
    expect(filters[0].options.filters[0].extensions).toContain("audio/*");
  });

  test("names a file that would not import", async ({ app }) => {
    const uri = "content://downloads/notes";
    await app.boot({
      ...ready(),
      "plugin:dialog|open": ok([uri]),
      import_audio: ok({
        tracks_loaded: 0,
        last_node_id: null,
        failures: [{ path: "/data/user/0/app.edytlab.android/projects/x/imports/notes.wav", error: "decode failed: unsupported format" }],
      }),
    });
    const page = app.page;
    await page.getByTestId("open-audio-button").tap();
    const banner = page.getByTestId("render-error");
    await expect(banner).toContainText("notes.wav");
    await expect(banner).toContainText("unsupported format");
  });
});

test.describe("projects on Android", () => {
  const PROJECTS = "/data/user/0/app.edytlab.android/projects";
  const existing = [
    { path: `${PROJECTS}/interview`, name: "Interview", last_opened_at: "2026-11-01T10:00:00Z" },
    { path: `${PROJECTS}/demo`, name: "Demo", last_opened_at: "2026-10-01T10:00:00Z" },
  ];

  test("lists the saved projects and opens one", async ({ app }) => {
    await app.boot({
      ...ready(),
      ...androidCommands({ projects: existing }),
      // `open_project` answers `ProjectInfo`: the folder and its head.
      open_project: ok({ path: existing[1].path, head: null }),
    });
    const page = app.page;
    await page.getByTestId("mobile-menu-button").tap();
    await page.getByTestId("menu-projects").tap();

    const sheet = page.getByTestId("projects-sheet");
    await expect(sheet.getByTestId("project-row")).toHaveCount(2);
    await sheet.getByText("Demo", { exact: true }).tap();

    await expect(sheet).toHaveCount(0);
    expect(await app.requestsFor("open_project")).toEqual([{ path: existing[1].path }]);
  });

  test("creates a project from a name, with no folder picker", async ({ app }) => {
    const made = `${PROJECTS}/episode-12`;
    await app.boot({
      ...ready(),
      create_managed_project: ok(made),
      open_project: ok({ path: made, head: null }),
    });
    const page = app.page;
    await page.getByTestId("mobile-menu-button").tap();
    await page.getByTestId("menu-projects").tap();
    await expect(page.getByTestId("projects-empty")).toBeVisible();

    await page.getByTestId("new-project-name").fill("Episode 12");
    await page.getByTestId("new-project-create").tap();

    await expect(page.getByTestId("projects-sheet")).toHaveCount(0);
    expect(await app.requestsFor("create_managed_project")).toEqual([{ name: "Episode 12" }]);
    expect(await app.requestsFor("open_project")).toEqual([{ path: made }]);
    // The desktop's folder dialog is never opened.
    expect(await app.requestsFor("plugin:dialog|open")).toEqual([]);
  });
});

test.describe("exports on Android", () => {
  const files = [{ name: "mix.wav", bytes: 3_400_000, modified_ms: Date.UTC(2026, 10, 14, 12) }];

  test("lists exported files and shares one", async ({ app }) => {
    await app.boot({
      ...ready(),
      ...androidCommands({ exports: files }),
      // `share_export` is `CmdResult<()>`: null once the sheet is up.
      share_export: ok(null),
    });
    const page = app.page;
    await page.getByTestId("mobile-menu-button").tap();
    await page.getByTestId("menu-exports").tap();

    const row = page.getByTestId("export-row");
    await expect(row).toContainText("mix.wav");
    await expect(row).toContainText("3.2 MB");
    await row.getByTestId("export-share").tap();
    await expect.poll(() => app.requestsFor("share_export")).toEqual([{ name: "mix.wav" }]);
  });

  test("saves one to the place the system dialog names", async ({ app }) => {
    const target = "content://com.android.externalstorage.documents/document/primary%3AMusic%2Fmix.wav";
    await app.boot({
      ...ready(),
      ...androidCommands({ exports: files }),
      // Android's save dialog answers the address of the document it made.
      "plugin:dialog|save": ok(target),
      save_export_to: ok(null),
    });
    const page = app.page;
    await page.getByTestId("mobile-menu-button").tap();
    await page.getByTestId("menu-exports").tap();
    await page.getByTestId("export-save").tap();

    await expect(page.getByTestId("exports-saved")).toContainText("mix.wav");
    expect(await app.requestsFor("save_export_to")).toEqual([{ name: "mix.wav", uri: target }]);
  });
});
