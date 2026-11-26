# VocaFlow

Context-aware dictation for macOS, Windows and Linux. Hold a shortcut, speak, let go: the text lands in whatever you're typing in, cleaned up and formatted for that app. Everything runs on your machine.

- **Local speech recognition** with Whisper (whisper.cpp). Four accuracy tiers (Low, Medium, High, Ultra), each its own model. Switching downloads the new model in the background and removes the old one once the new one is ready. Dictation keeps working throughout.
- **Context-aware formatting** with a small local model (Qwen2.5 1.5B Instruct). VocaFlow reads which app and field you're in and the text around the cursor, then formats for it: an email body in Mail, a commit message after `git commit -m "`, a post on X, a message in Slack. It removes filler words, applies self-corrections ("at 3, actually 4"), handles spoken punctuation, and never uses em dashes or emojis. You can turn it off.
- **The pill**: a small status pill at the bottom of the screen, matching the Figma file. It's real Liquid Glass on macOS (NSGlassEffectView, with a vibrancy fallback before macOS 26) and an acrylic tint on Windows and Linux, in light and dark.
- **Shortcut**: `Alt+D` or `F9` by default, or any combination you choose. Hold to talk; tap for hands-free and tap again to stop; `Esc` cancels. Hands-free stops on its own after 30 seconds of silence.
- **Daily-driver details**: VocaFlow restores your clipboard after pasting, types instead of pasting in apps you list, picks the right paste shortcut in Linux terminals, and matches spacing and capitalization to the text already in the field. It also has a custom dictionary, the last 50 dictations in History, a microphone picker with a live level meter, optional start and stop sounds, and launch at login.

## Stack

- Tauri 2 (Rust) + React 19 + Vite + Tailwind v4
- UI: [krts.ui](https://www.npmjs.com/package/krts.ui), re-tinted for VocaFlow; wordmark set in [Allo GR](https://github.com/l12-krts/Allo-GR)
- Speech: `whisper-rs` (whisper.cpp, Metal on Apple Silicon)
- Formatter: `candle` with a vendored quantized Qwen2 that caches the fixed prompt prefix per target, so each dictation only processes its own words
- Audio: `cpal` + `rubato`; input: `enigo` + `arboard`
- Context: macOS Accessibility (AX), Windows UI Automation, Linux AT-SPI

## Develop

```bash
npm install
```

```bash
npm run tauri dev
```

The UI also runs in a plain browser (`npm run dev`) against an in-memory mock backend. Useful query parameters:

- `?onboarded=0` shows onboarding
- `?os=macos|windows|linux` switches the platform styling
- `?theme=dark` forces dark mode
- `pill.html?demo&bg` loops the pill through a dictation

### Linux build dependencies

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libxdo-dev libssl-dev cmake clang
```

### Engine check without the UI

```bash
cd src-tauri && cargo run --release --example e2e -- speech.wav ./models medium format
```

This downloads the tier and the formatter, transcribes a 16-bit WAV, and prints the formatted result for several app contexts. Pass `text:...` instead of a WAV path to test formatting only.

## Platform notes

| | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Pill surface | Liquid Glass (26+), vibrancy (13–15) | Figma acrylic tint | Figma acrylic tint |
| Shortcut | Global | Global | X11 global; Wayland via the GlobalShortcuts portal (GNOME 46+, KDE 6) |
| Paste | Cmd+V | Ctrl+V | Ctrl+V, Ctrl+Shift+V in terminals; Wayland needs `wtype` or `ydotool` |
| Context | Accessibility permission | UI Automation | AT-SPI (accessibility bus) |
| Acceleration | Metal | AVX2 | AVX2 |

On macOS, VocaFlow asks for Microphone and Accessibility permission during onboarding. Accessibility is what lets it paste and read the field you're typing in.

## Models

| Tier | Model | Download |
| --- | --- | --- |
| Low | Whisper base (q5_1) | 60 MB |
| Medium | Whisper small (q5_1) | 190 MB |
| High | Whisper large-v3-turbo (q5_0) | 574 MB |
| Ultra | Whisper large-v3 (q5_0) | 1.1 GB |
| Formatter | Qwen2.5 1.5B Instruct (Q4_K_M) | 1.1 GB |

Models download from Hugging Face into the app data directory. Downloads resume after interruptions, and a stalled connection is retried.

## CI

`.github/workflows/build.yml` runs typecheck, the frontend build and the Rust tests, then builds unsigned installers on every push: `.dmg` (Apple Silicon and Intel), `.msi`/`.exe`, AppImage and `.deb`. ggml is built for a portable AVX2 baseline rather than the CI machine's CPU.
