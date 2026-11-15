#!/usr/bin/env bash
# Install the Android APK on the running emulator and drive the real app.
#
# Run by .github/workflows/android.yml inside
# reactivecircus/android-emulator-runner, which boots the emulator and waits
# for it before this starts. It can be run by hand the same way against any
# device `adb` sees, as long as the APK is a debug build (the checks reach
# the app through its WebView's debugging socket and `run-as`):
#
#   bash scripts/ci/android-smoke.sh <dir holding the APK>
#
# What it checks is in android-smoke.mjs, in three phases with this script's
# work between them: the app starts and answers; audio pushed into a project
# loads, draws, plays, exports and records; and after the app is stopped and
# started again, the saved key and the project are still there.
#
# What it leaves in ./smoke-out, which the workflow uploads whether the run
# passed or not: screenshots, the whole logcat, and what each phase printed.
set -uo pipefail

apk_dir="${1:?usage: android-smoke.sh <dir holding the APK>}"
pkg="app.edytlab.android"
out="smoke-out"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
mkdir -p "$out"
rm -f "$out/smoke-state.json"

# The universal build if there is one, else the one for the emulator's ABI.
apk="$(ls "$apk_dir"/*universal*.apk 2>/dev/null | head -n1 || true)"
if [ -z "$apk" ]; then
  apk="$(ls "$apk_dir"/*x86_64*.apk 2>/dev/null | head -n1 || true)"
fi
if [ -z "$apk" ]; then
  apk="$(ls "$apk_dir"/*.apk | head -n1)"
fi
echo "APK: $apk ($(du -h "$apk" | cut -f1))"

fail=0

# Start the app and wait until it has stayed up: a native crash at start (a
# missing library, a panic in setup) takes the process with it within a
# second or two.
launch() {
  adb shell am start -W -n "$pkg/.MainActivity" | tee -a "$out/am-start.txt"
  local alive=0
  for _ in $(seq 1 15); do
    if adb shell pidof "$pkg" > /dev/null 2>&1; then alive=1; else alive=0; fi
    sleep 1
  done
  if [ "$alive" != 1 ]; then
    echo "FAIL: $pkg is not running 15 s after launch"
    return 1
  fi
}

# Run one phase of the in-app checks against the app that is running now.
phase() {
  local name="$1" pid
  pid="$(adb shell pidof "$pkg" | tr -d '\r' | awk '{print $1}')"
  adb forward --remove tcp:9222 > /dev/null 2>&1 || true
  adb forward tcp:9222 "localabstract:webview_devtools_remote_${pid}"
  if ! node "$here/android-smoke.mjs" 9222 "$out" "$name" 2>&1 | tee "$out/in-app-checks-$name.txt"; then
    echo "FAIL: in-app checks, phase $name"
    fail=1
  fi
  adb exec-out screencap -p > "$out/screen-$name.png" 2>/dev/null || true
}

# Copy a file from this machine into the app's own storage. Through a pipe
# into `run-as`, because the app's storage is the app's alone and a debug
# build lets `run-as` act as it.
into_app() {
  local from="$1" to="$2"
  adb shell "run-as $pkg mkdir -p '$(dirname "$to")'"
  adb push "$from" /data/local/tmp/smoke-upload > /dev/null
  adb shell "cat /data/local/tmp/smoke-upload | run-as $pkg sh -c 'cat > \"$to\"'"
  adb shell rm -f /data/local/tmp/smoke-upload
}

adb logcat -c || true
adb install -r "$apk" || { echo "FAIL: the APK did not install"; exit 1; }

# Granted up front, so the checks never meet a system permission dialog.
adb shell pm grant "$pkg" android.permission.RECORD_AUDIO 2>/dev/null || true

if launch; then
  adb exec-out screencap -p > "$out/screen-launch.png" || true
  phase start

  project="$(node -e 'try { process.stdout.write(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).project || "") } catch {}' "$out/smoke-state.json")"
  if [ -n "$project" ]; then
    # Two seconds of a 440 Hz tone at half scale, 44.1 kHz mono 16-bit: what
    # the `audio` phase expects of `tone.wav`.
    node -e '
      const rate = 44100, n = rate * 2, data = Buffer.alloc(n * 2);
      for (let i = 0; i < n; i++) data.writeInt16LE(Math.round(Math.sin(2 * Math.PI * 440 * i / rate) * 0.5 * 32767), i * 2);
      const h = Buffer.alloc(44);
      h.write("RIFF", 0); h.writeUInt32LE(36 + data.length, 4); h.write("WAVEfmt ", 8);
      h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22);
      h.writeUInt32LE(rate, 24); h.writeUInt32LE(rate * 2, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34);
      h.write("data", 36); h.writeUInt32LE(data.length, 40);
      require("fs").writeFileSync(process.argv[1], Buffer.concat([h, data]));
    ' "$out/tone.wav"
    into_app "$out/tone.wav" "$project/imports/tone.wav"
    # The decoder crate's own M4A fixture: half a second of the same tone.
    into_app "$repo/crates/audio-decoder/tests/fixtures/tone_440_mono_44100.m4a" \
      "$project/imports/tone_440_mono_44100.m4a"
    phase audio

    # Stop the app outright and start it again: what was only in memory is
    # gone, and what was saved has to be read back.
    adb shell am force-stop "$pkg"
    sleep 2
    if launch; then
      phase restart
      # Back at the top of the app puts it behind the launcher and leaves it
      # running.
      adb shell input keyevent 4
      sleep 2
      if adb shell pidof "$pkg" > /dev/null 2>&1; then
        echo "ok   Back left the app running"
      else
        echo "FAIL: the app died on Back"
        fail=1
      fi
    else
      fail=1
    fi
  else
    echo "FAIL: the first phase made no project, so the audio checks did not run"
    fail=1
  fi
else
  fail=1
fi

adb exec-out screencap -p > "$out/screen-final.png" 2>/dev/null || true
adb logcat -d > "$out/logcat.txt" 2>/dev/null || true
# A crash the checks above did not notice still fails the run.
# Only the app's own: an emulator's system services crash on their own account.
if grep -E "UnsatisfiedLinkError|panicked at|>>> ${pkg} <<<|Process: ${pkg}" "$out/logcat.txt" > "$out/crashes.txt"; then
  echo "FAIL: the log has a crash:"
  head -n 40 "$out/crashes.txt"
  fail=1
fi

if [ "$fail" != 0 ]; then
  exit 1
fi
echo "OK: $pkg started, passed its checks, and kept its key and project across a restart"
