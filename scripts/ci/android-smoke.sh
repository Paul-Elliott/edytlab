#!/usr/bin/env bash
# Install the Android APK on the running emulator, start the app and check
# that it is alive and drew its window.
#
# Run by .github/workflows/android.yml inside
# reactivecircus/android-emulator-runner, which boots the emulator and waits
# for it before this starts. It can be run by hand the same way against any
# device `adb` sees:
#
#   bash scripts/ci/android-smoke.sh <dir holding the APK>
#
# What it leaves in ./smoke-out, which the workflow uploads whether the run
# passed or not: a screenshot, the whole logcat, and what the in-app checks
# (android-smoke.mjs) printed.
set -euo pipefail

apk_dir="${1:?usage: android-smoke.sh <dir holding the APK>}"
pkg="app.edytlab.android"
out="smoke-out"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$out"

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
finish() {
  adb logcat -d > "$out/logcat.txt" 2>/dev/null || true
  adb exec-out screencap -p > "$out/screen-final.png" 2>/dev/null || true
}
trap finish EXIT

adb logcat -c || true
adb install -r "$apk"

# Granted up front, so the checks never meet a system permission dialog.
adb shell pm grant "$pkg" android.permission.RECORD_AUDIO 2>/dev/null || true

adb shell am start -W -n "$pkg/.MainActivity" | tee "$out/am-start.txt"

# The window has to come up and stay up: a native crash at start (a missing
# library, a panic in setup) takes the process with it within a second.
alive=0
for _ in $(seq 1 30); do
  if adb shell pidof "$pkg" > /dev/null 2>&1; then
    alive=1
  else
    alive=0
  fi
  sleep 1
done
adb exec-out screencap -p > "$out/screen-start.png" || true
if [ "$alive" != 1 ]; then
  echo "FAIL: $pkg is not running 30 s after launch"
  fail=1
fi

# The checks inside the app, over the WebView's debugging socket (a debug
# build opens one). Node only: no browser and no npm install.
if [ "$fail" = 0 ] && [ -f "$here/android-smoke.mjs" ]; then
  pid="$(adb shell pidof "$pkg" | tr -d '\r')"
  adb forward tcp:9222 "localabstract:webview_devtools_remote_${pid}"
  if ! node "$here/android-smoke.mjs" 9222 "$out" 2>&1 | tee "$out/in-app-checks.txt"; then
    echo "FAIL: in-app checks"
    fail=1
  fi
  adb forward --remove tcp:9222 || true
fi

adb logcat -d > "$out/logcat.txt" || true
# A crash the checks above did not notice still fails the run.
if grep -E "FATAL EXCEPTION|UnsatisfiedLinkError|Fatal signal [0-9]+ .*${pkg}|panicked at" "$out/logcat.txt" | grep -v "chromium" > "$out/crashes.txt"; then
  echo "FAIL: the log has a crash:"
  head -n 40 "$out/crashes.txt"
  fail=1
fi

if [ "$fail" != 0 ]; then
  exit 1
fi
echo "OK: $pkg started, stayed up and passed its checks"
