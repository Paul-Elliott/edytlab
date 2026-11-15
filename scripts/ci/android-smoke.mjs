// In-app checks for the Android smoke run (scripts/ci/android-smoke.sh).
//
// Talks to the app's WebView over the Chrome DevTools Protocol, which a
// debug build exposes on a local socket that the shell script forwards to a
// TCP port. Every check runs inside the real app: the page is the shipped
// frontend, and `invoke` reaches the real Rust commands on the device.
//
// The run has three phases, because two things need the shell script
// between them (files pushed into the app's storage, and a restart):
//
//   node android-smoke.mjs <port> <out dir> start     the app is up; make a project
//   node android-smoke.mjs <port> <out dir> audio     fixtures are in it; load, draw, play, export, record
//   node android-smoke.mjs <port> <out dir> restart   after a force-stop: what must have survived
//
// Node 22 only (global `fetch` and `WebSocket`); nothing to install.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const port = Number(process.argv[2] ?? 9222);
const outDir = process.argv[3] ?? ".";
const phase = process.argv[4] ?? "start";

/** What one phase leaves for the next, and for the shell script. */
const statePath = join(outDir, "smoke-state.json");
const state = existsSync(statePath) ? JSON.parse(readFileSync(statePath, "utf8")) : {};
const saveState = () => writeFileSync(statePath, JSON.stringify(state, null, 2));

/** A key in the shape Settings accepts, that names what it is. Never sent anywhere. */
const SMOKE_KEY = "sk-ant-smoke-test-not-a-real-key-0000000000000000";

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
async function evaluate(cdp, expression, { userGesture = false } = {}) {
  const { result, exceptionDetails } = await cdp.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
    userGesture,
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

/** The window has mounted the app. Every phase starts from this. */
async function windowDraws(cdp) {
  let children = 0;
  for (let i = 0; i < 30 && children === 0; i++) {
    children = await evaluate(cdp, `document.getElementById("root")?.childElementCount ?? 0`);
    if (children === 0) await sleep(1000);
  }
  assert(children > 0, "#root is empty: the frontend did not mount");
}

// Each check is named for what a person would see break. They run in order
// and all of them run: one failure does not hide the next.
const phases = {
  start: [
    ["the window draws the app, in its Android layout", async (cdp) => {
      await windowDraws(cdp);
      const platform = await evaluate(cdp, `document.documentElement.dataset.platform`);
      assert(platform === "android", `data-platform is ${JSON.stringify(platform)}`);
      const header = await evaluate(cdp, `!!document.querySelector('[data-testid="mobile-header"]')`);
      assert(header, "the Android header is not on screen");
      const size = await evaluate(cdp, `({ w: innerWidth, h: innerHeight, sw: document.documentElement.scrollWidth })`);
      assert(size.sw <= size.w, `the page is wider than the screen: ${JSON.stringify(size)}`);
      return JSON.stringify(size);
    }],
    ["the Rust side answers commands", async (cdp) => {
      const providers = await invoke(cdp, "list_providers");
      assert(Array.isArray(providers) && providers.length > 0, `list_providers answered ${JSON.stringify(providers)}`);
    }],
    ["the device says what it is", async (cdp) => {
      const device = await invoke(cdp, "device_info");
      assert(device.android === true, `device_info.android is ${device.android}`);
      assert(device.total_ram_bytes > 500_000_000, `total_ram_bytes is ${device.total_ram_bytes}`);
      return `${device.model}, ${(device.total_ram_bytes / 1e9).toFixed(1)} GB, stems ${device.stems_supported ? "offered" : "not offered"}`;
    }],
    ["the bundled templates and licence text were unpacked", async (cdp) => {
      const templates = await invoke(cdp, "list_templates");
      assert(Array.isArray(templates) && templates.length >= 3, `list_templates answered ${JSON.stringify(templates)}`);
      const notices = await invoke(cdp, "third_party_notices");
      assert(typeof notices === "string" && notices.length > 10_000, `the notices are ${String(notices).length} characters`);
      return `${templates.length} templates, ${notices.length} characters of notices`;
    }],
    ["the bundled skills install", async (cdp) => {
      const written = await invoke(cdp, "install_bundled_skills");
      assert(typeof written === "number", `install_bundled_skills answered ${JSON.stringify(written)}`);
      const skills = await invoke(cdp, "list_skills");
      assert(Array.isArray(skills) && skills.length >= 10, `list_skills has ${skills?.length} entries`);
      return `${skills.length} skills`;
    }],
    ["the ONNX Runtime library in the package is found and loads", async (cdp) => {
      const status = await invoke(cdp, "ml_runtime_status");
      const text = JSON.stringify(status);
      assert(/1\.23\.2/.test(text), `ml_runtime_status answered ${text}`);
      assert(/bundled/i.test(text), `the runtime is not the bundled one: ${text}`);
      return text;
    }],
    // The smallest of the three models, from Hugging Face, all the way to
    // "ready": the lock file, the writes into the app's storage, the digest
    // check and the rename. A first device test found the lock refused on
    // Android before a byte was asked for, which no host test could see.
    ["a model downloads into the app's storage", async (cdp) => {
      const model = "speaker-diarization";
      let status;
      for (let attempt = 1; attempt <= 2; attempt++) {
        status = await invoke(cdp, "download_ml_model", { model });
        for (let i = 0; i < 300 && status.state === "downloading"; i++) {
          await sleep(1000);
          status = await invoke(cdp, "ml_model_status", { model });
        }
        // One more try covers a dropped connection; a fault in the app
        // fails the second time as it did the first.
        if (status.state !== "failed") break;
        console.log(`     attempt ${attempt}: ${status.reason}`);
      }
      assert(status.state === "ready", `the download ended as ${JSON.stringify(status)}`);
      assert(status.bytes > 1_000_000, `the model on disk is ${status.bytes} bytes`);
      return `${model}, ${(status.bytes / 1e6).toFixed(1)} MB`;
    }],
    ["an API key is saved to the Android Keystore", async (cdp) => {
      const landed = await invoke(cdp, "set_api_key_for", { provider: "anthropic", key: SMOKE_KEY });
      assert(landed?.persistent === true, `the save answered ${JSON.stringify(landed)}`);
      assert((await invoke(cdp, "has_api_key_for", { provider: "anthropic" })) === true, "the key just saved is not there");
      const persistence = await invoke(cdp, "get_keychain_persistence");
      assert(persistence?.persistent === true, `get_keychain_persistence answered ${JSON.stringify(persistence)}`);
    }],
    ["a project is made from a name and opened", async (cdp) => {
      const path = await invoke(cdp, "create_managed_project", { name: "Smoke test" });
      assert(typeof path === "string" && path.includes("/projects/"), `create_managed_project answered ${JSON.stringify(path)}`);
      const info = await invoke(cdp, "open_project", { path });
      assert(info?.path === path, `open_project answered ${JSON.stringify(info)}`);
      const listed = await invoke(cdp, "list_managed_projects");
      assert(listed.some((p) => p.path === path && p.name === "Smoke test"), `the project is not listed: ${JSON.stringify(listed)}`);
      state.project = path;
      saveState();
      return path;
    }],
  ],

  audio: [
    ["the window is up", windowDraws],
    ["a WAV in the project loads as a track", async (cdp) => {
      const wav = `${state.project}/imports/tone.wav`;
      const result = await invoke(cdp, "batch_load", { paths: [wav] });
      assert(result.tracks_loaded === 1, `batch_load answered ${JSON.stringify(result)}`);
      const tracks = await invoke(cdp, "list_tracks");
      assert(tracks.length === 1 && tracks[0].audio_path, `list_tracks answered ${JSON.stringify(tracks)}`);
      state.head = result.last_node_id;
      state.trackAudio = tracks[0].audio_path;
      saveState();
    }],
    ["the timeline's outline comes from the backend", async (cdp) => {
      const outline = await invoke(cdp, "waveform_peaks", { path: state.trackAudio });
      assert(Math.abs(outline.duration_sec - 2) < 0.05, `duration is ${outline.duration_sec}`);
      assert(outline.peaks.length >= 350 && outline.peaks.length <= 450, `${outline.peaks.length} peaks`);
      const loudest = Math.max(...outline.peaks) / 32767;
      assert(Math.abs(loudest - 0.5) < 0.05, `the loudest peak is ${loudest}, the tone is 0.5`);
      // And not for a file outside the project.
      let refused = false;
      try {
        await invoke(cdp, "waveform_peaks", { path: "/system/build.prop" });
      } catch {
        refused = true;
      }
      assert(refused, "waveform_peaks read a file outside the project");
      return `${outline.peaks.length} peaks`;
    }],
    ["an M4A decodes with Android's own decoder", async (cdp) => {
      const m4a = `${state.project}/imports/tone_440_mono_44100.m4a`;
      const result = await invoke(cdp, "batch_load", { paths: [m4a] });
      assert(result.tracks_loaded === 1, `batch_load answered ${JSON.stringify(result)}`);
      const tracks = await invoke(cdp, "list_tracks");
      assert(tracks.length === 2, `there are ${tracks.length} tracks`);
      const outline = await invoke(cdp, "waveform_peaks", { path: tracks[1].audio_path });
      // The fixture is half a second of a 440 Hz tone at half scale.
      assert(Math.abs(outline.duration_sec - 0.5) < 0.06, `the M4A is ${outline.duration_sec} s long`);
      const loudest = Math.max(...outline.peaks) / 32767;
      assert(loudest > 0.3 && loudest < 0.7, `the M4A's loudest peak is ${loudest}`);
      state.head = result.last_node_id;
      saveState();
      return `${outline.duration_sec.toFixed(3)} s, peak ${loudest.toFixed(2)}`;
    }],
    ["the mix renders and the WebView plays it from the project", async (cdp) => {
      const preview = await invoke(cdp, "render_preview", { node: state.head });
      const path = typeof preview === "string" ? preview : preview?.path;
      assert(typeof path === "string" && path.endsWith(".wav"), `render_preview answered ${JSON.stringify(preview)}`);
      const played = await evaluate(
        cdp,
        `(async () => {
          const audio = new Audio(window.__TAURI_INTERNALS__.convertFileSrc(${JSON.stringify(path)}));
          audio.muted = true;
          await new Promise((resolve, reject) => {
            audio.addEventListener("loadedmetadata", resolve, { once: true });
            audio.addEventListener("error", () => reject(new Error("the audio element could not load the file")), { once: true });
          });
          const duration = audio.duration;
          audio.currentTime = 1;
          await audio.play();
          await new Promise((r) => setTimeout(r, 600));
          const at = audio.currentTime;
          audio.pause();
          return { duration, at };
        })()`,
        { userGesture: true },
      );
      assert(Math.abs(played.duration - 2) < 0.1, `the mix is ${played.duration} s long`);
      assert(played.at > 1.2, `after a seek to 1 s and 0.6 s of play the element is at ${played.at} s`);
      return JSON.stringify(played);
    }],
    ["an export lands in the exports folder", async (cdp) => {
      const out = await invoke(cdp, "export_destination", { name: "smoke.wav" });
      assert(out.endsWith("/exports/smoke.wav"), `export_destination answered ${out}`);
      await invoke(cdp, "render_range", { nodeId: state.head, startSec: 0, endSec: 1, outPath: out });
      const files = await invoke(cdp, "list_exports");
      const file = files.find((f) => f.name === "smoke.wav");
      assert(file && file.bytes > 40_000, `the exports are ${JSON.stringify(files)}`);
      return `${file.bytes} bytes`;
    }],
    ["the microphone opens and a take is saved into the project", async (cdp) => {
      assert((await invoke(cdp, "ensure_microphone")) === true, "the microphone is not allowed (the script grants it)");
      await invoke(cdp, "start_recording");
      await sleep(1500);
      const take = await invoke(cdp, "stop_recording", { outputPath: "smoke_take.wav" });
      assert(String(take.path).startsWith(`${state.project}/imports/`), `the take was saved at ${take.path}`);
      assert(take.sample_rate >= 8000 && take.channels >= 1, `the take is ${JSON.stringify(take)}`);
      const outline = await invoke(cdp, "waveform_peaks", { path: take.path });
      assert(outline.duration_sec > 0.8, `the take is ${outline.duration_sec} s long`);
      return `${outline.duration_sec.toFixed(2)} s at ${take.sample_rate} Hz`;
    }],
  ],

  restart: [
    ["the window comes back up", windowDraws],
    ["the saved key survived the app being stopped", async (cdp) => {
      // The startup reads are off the setup thread; the command waits for them.
      assert((await invoke(cdp, "has_api_key_for", { provider: "anthropic" })) === true, "the key is gone after a restart");
      await invoke(cdp, "clear_api_key_for", { provider: "anthropic" });
      assert((await invoke(cdp, "has_api_key_for", { provider: "anthropic" })) === false, "the key could not be removed");
    }],
    ["the project and its audio are still there", async (cdp) => {
      const listed = await invoke(cdp, "list_managed_projects");
      assert(listed.some((p) => p.path === state.project), `the project is not listed: ${JSON.stringify(listed)}`);
      await invoke(cdp, "open_project", { path: state.project });
      const tracks = await invoke(cdp, "list_tracks");
      assert(tracks.length === 2, `the project reopened with ${tracks.length} tracks`);
      const exports = await invoke(cdp, "list_exports");
      assert(exports.some((f) => f.name === "smoke.wav"), "the export is gone");
    }],
    ["the downloaded model is still there", async (cdp) => {
      const status = await invoke(cdp, "ml_model_status", { model: "speaker-diarization" });
      assert(status.state === "ready", `after a restart the model is ${JSON.stringify(status)}`);
    }],
  ],
};

const checks = phases[phase];
if (!checks) throw new Error(`unknown phase ${phase}`);

const page = await findPage();
console.log(`[${phase}] page: ${page.url}`);
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
writeFileSync(join(outDir, `in-app-checks-${phase}.json`), JSON.stringify(results, null, 2));
process.exit(results.every((r) => r.ok) ? 0 : 1);
