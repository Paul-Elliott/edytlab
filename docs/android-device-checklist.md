# Android device checklist

The automated checks run in a browser and on an emulator. They cannot judge real hardware. This list is what to try on a phone or tablet, in order, and what to report if a step fails.

Install: copy the APK to the device, open it, and allow "install unknown apps" for the app you opened it with. A newer APK from this fork installs over an older one and keeps your projects.

## 1. First launch

- [ ] The app opens to a "Welcome" sheet asking for a provider and an API key.
- [ ] The header sits below the status bar, and nothing is hidden behind the navigation bar or a notch.
- [ ] Paste a key, tap **Test**, then **Save & Continue**.
- [ ] Close the app from the recent apps list and open it again. It must not ask for the key again.

## 2. Import and play

- [ ] Tap **Import** and pick a song (MP3 or M4A). A track appears on the Timeline tab with a waveform.
- [ ] Tap Play. You hear the audio. Tap the waveform to move the playhead.
- [ ] Drag a finger along the waveform: a range is selected. Tap elsewhere: it clears.
- [ ] Pinch with two fingers on the waveform: it zooms. The page itself must not zoom.
- [ ] If a file type is greyed out in the picker, note the type and which file manager showed it.

## 3. Edit by chat

- [ ] On the Chat tab, type "make this 6 dB louder" and send. The keyboard must not cover the text box.
- [ ] The reply arrives, and the Timeline shows the change. Undo and redo (the arrows in the header) work.
- [ ] Open **History**. Press and hold a step: a menu opens with "Set as head".

## 4. Stem separation (the main one)

- [ ] Menu, **Settings**, Account: the **Stem model** row offers Download (about 171 MB). On a device under 8 GB it should say stems are not offered.
- [ ] Download it on Wi-Fi. The screen should stay on while it downloads.
- [ ] Ask the assistant: "separate this track into stems". Note how long a 3 to 4 minute song takes.
- [ ] Four new tracks appear (vocals, drums, bass, other). Solo each and play.
- [ ] If the app closes during separation, that is Android running out of memory. Note the song length and the device model.

## 5. Export

- [ ] Ask the assistant to "export the mix as MP3". Then open Menu, **Exports**: the file is listed.
- [ ] **Share** opens the Android share sheet. **Save to...** writes the file where you choose, and it plays in another app.

## 6. Projects

- [ ] Menu, **Projects**: create a project by name, import something, then switch back to the first project. Each keeps its own audio.
- [ ] Duplicate a project, and delete one you do not need.

## 7. Coming back to the app

- [ ] With a project open and a few messages in the chat, swipe the app away from the recent apps list, then open it again. It opens on the same project, with its tracks, and the chat shows the earlier messages.
- [ ] Type "continue" or ask about something said before. The assistant answers as someone who remembers the conversation.
- [ ] Switch to another project and back. Each project shows its own chat.
- [ ] The first screen (Timeline tab, no audio) fits upright: four buttons, none off the side, and no "Ctrl+O".

## 8. Transcription (optional)

- [ ] Download the **Speech model** (about 139 MB), ask "transcribe track 1", and open the Transcript tab.
- [ ] Tap a word, tap another, tap **Cut**. The audio under those words is removed.

## 9. Recording (least tested)

- [ ] Tap the red Record button. Android asks for the microphone once. Speak, then stop: a new track appears.
- [ ] Recording stops if you leave the app. Latency calibration and punch-in are not offered in the Android header.

## Known limits

- Sign in with a ChatGPT plan is not offered on Android. Use an API key.
- MCP servers that start a local program cannot run on Android. Remote (HTTP) servers can.
- No spectrogram view and no snap-to-zero on Android.
- Long work stops if the app is put in the background for long. Keep it open during stem separation.
- A local Ollama address over plain `http://` is blocked in the release APK.

## What to send back if something fails

The step number, what you saw, the device model and Android version, and for a crash the length of the audio you were working on.
