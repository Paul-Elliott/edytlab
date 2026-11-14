// In-app checks for the Android smoke run (scripts/ci/android-smoke.sh).
//
// Talks to the app's WebView over the Chrome DevTools Protocol, which a
// debug build exposes on a local socket that the shell script forwards to a
// TCP port. Every check runs inside the real app: the page is the shipped
// frontend, and `invoke` reaches the real Rust commands on the device.
//
// Node 22 only (global `fetch` and `WebSocket`); nothing to install.
//
//   node android-smoke.mjs <port> <out dir>

import { writeFileSync } from "node:fs";
import { join } from "node:path";

const port = Number(process.argv[2] ?? 9222);
const outDir = process.argv[3] ?? ".";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The app's page, once the WebView lists it. */
async function findPage() {
  let last = "";
  for (let i = 0; i < 60; i++) {
    try {
      const pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      last = JSON.stringify(pages.map((p) => p.url));
      const page = pages.find(
        (p) => p.type === "page" && /tauri\.localhost|localhost/.test(p.url),
      );
      if (page) return page;
    } catch (e) {
      last = String(e);
    }
    await sleep(1000);
  }
  throw new Error(`no app page on the debugging port after 60 s (${last})`);
}

/** A minimal CDP client: one socket, calls matched to answers by id. */
function connect(url) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    let next = 1;
    const waiting = new Map();
    ws.onerror = () => reject(new Error(`could not open ${url}`));
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      const entry = waiting.get(msg.id);
      if (!entry) return;
      waiting.delete(msg.id);
      if (msg.error) entry.reject(new Error(msg.error.message));
      else entry.resolve(msg.result);
    };
    ws.onopen = () =>
      resolve({
        send(method, params = {}) {
          const id = next++;
          return new Promise((res, rej) => {
            waiting.set(id, { resolve: res, reject: rej });
            ws.send(JSON.stringify({ id, method, params }));
          });
        },
        close: () => ws.close(),
      });
  });
}

/** Evaluate `expression` in the page and return its value (awaited). */
async function evaluate(cdp, expression) {
  const { result, exceptionDetails } = await cdp.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (exceptionDetails) {
    const text =
      exceptionDetails.exception?.description ??
      exceptionDetails.exception?.value ??
      exceptionDetails.text;
    throw new Error(String(text));
  }
  return result.value;
}

/** Call a Tauri command from the page, as the app's own code does. */
const invoke = (cdp, cmd, args = {}) =>
  evaluate(
    cdp,
    `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)})`,
  );

const assert = (ok, message) => {
  if (!ok) throw new Error(message);
};

// Each check is named for what a person would see break. They run in order
// and all of them run: one failure does not hide the next.
const checks = [
  [
    "the window draws the app",
    async (cdp) => {
      let children = 0;
      for (let i = 0; i < 30 && children === 0; i++) {
        children = await evaluate(
          cdp,
          `document.getElementById("root")?.childElementCount ?? 0`,
        );
        if (children === 0) await sleep(1000);
      }
      assert(children > 0, "#root is empty: the frontend did not mount");
    },
  ],
  [
    "the Rust side answers commands",
    async (cdp) => {
      const providers = await invoke(cdp, "list_providers");
      assert(
        Array.isArray(providers) && providers.length > 0,
        `list_providers answered ${JSON.stringify(providers)}`,
      );
    },
  ],
  [
    "a default project is open",
    async (cdp) => {
      const tracks = await invoke(cdp, "list_tracks");
      assert(Array.isArray(tracks), `list_tracks answered ${JSON.stringify(tracks)}`);
    },
  ],
];

const page = await findPage();
console.log(`page: ${page.url}`);
const cdp = await connect(page.webSocketDebuggerUrl);
await cdp.send("Runtime.enable");

const results = [];
for (const [name, run] of checks) {
  try {
    const note = await run(cdp);
    results.push({ name, ok: true, note: note ?? null });
    console.log(`ok   ${name}${note ? `: ${note}` : ""}`);
  } catch (e) {
    results.push({ name, ok: false, error: String(e.message ?? e) });
    console.log(`FAIL ${name}: ${e.message ?? e}`);
  }
}
cdp.close();
writeFileSync(join(outDir, "in-app-checks.json"), JSON.stringify(results, null, 2));
process.exit(results.every((r) => r.ok) ? 0 : 1);
