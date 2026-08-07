# Gaze

[日本語](README.ja.md)

An expressive xeyes-style desktop application built with egui / eframe.

## Features

- Two eyes follow the desktop mouse cursor, trailing it the way eyes do: a lazy pursuit for small movements and a fast saccade for large ones
- The eyes squint as the cursor moves farther away, reaching a full squint at half the monitor diagonal, and the iris slides under the eyelid, which trims it along the curve of the eye
- The iris scales with both the width and the height of the window
- Blinks with naturally distributed intervals from 3 to 13 seconds, about nine times a minute
- A double blink follows one blink in sixteen, roughly once every two minutes
- Every couple of minutes one blink is a slow, deliberate close lasting almost a second, never two of them within 90 seconds
- Blinking can be set to standard, slow (about half as often, near the rate of someone reading), or off altogether
- After 3 minutes without mouse or keyboard input, checks for a yawn every 10 seconds with `x = idle seconds / 10` and probability `y = (x² - 324) / 576`; closes its eyes and sleeps after 5 minutes
- Transparent background with no title bar or taskbar entry
- Show, hide, and exit controls in the Windows system tray
- Optional automatic startup when signing in, on Windows and macOS alike
- Resizable by dragging the outer edges of the eyes, on Windows and macOS alike
- Optional always-on-top window level
- Follows you between desktops on macOS, including over a full-screen app
- Opacity selectable between 100% and 25%
- Restores the previous window position and size at startup, on Windows
- Remembers the always-on-top, show-on-all-desktops, blinking and opacity settings between runs
- DPI scaling support
- Automatically recovers stalled eye rendering with native Windows repaint notifications

Drag near the center of either eye to move the window. Drag the left or right outer edge to resize it horizontally, or the top or bottom edge to resize it vertically. The window runs from 120x80 to 540x320 points, and the face is laid out from whatever size you leave it at.

On Windows, left-click or right-click the tray icon to open the show/hide, always-on-top, blinking, opacity, automatic startup, and exit menu. Elsewhere there is no tray icon, so right-click either eye to open the always-on-top, show-on-all-desktops, start-at-login, blinking, opacity, and quit menu.

Showing Gaze on every desktop is on by default and is a macOS setting only: Windows has no supported API for pinning a window across its virtual desktops, so the tray menu does not offer it.

Automatic startup is configured only for the signed-in user and does not require administrator privileges. Windows uses its per-user `Run` registry key; macOS writes a LaunchAgent to `~/Library/LaunchAgents/org.spumoni.gaze.plist` that points at the running executable, so moving or rebuilding Gaze elsewhere means switching the setting off and on again. The agent is not loaded when you tick the box — it takes effect at your next login, rather than starting a second Gaze straight away.

The always-on-top, show-on-all-desktops, blinking and opacity settings are stored in `%APPDATA%\Gaze\settings.conf` on Windows, `~/Library/Application Support/Gaze/settings.conf` on macOS, and `$XDG_CONFIG_HOME/Gaze/settings.conf` elsewhere.

On Windows, idle detection counts only actual mouse cursor movement and Raw Input key presses. Touch, pen, other HID activity, and timestamp-only input updates do not reset the idle timer.

Release notes live in [CHANGELOG.md](CHANGELOG.md).

## Running

```sh
cargo run --release
```

On Windows and macOS, Gaze tracks the cursor across the whole desktop. On other operating systems, it currently tracks the cursor only while it is inside the window, so the eyes hardly ever squint there.

## Releasing on macOS

`scripts/macos-release.sh` builds a universal binary, wraps it in `Gaze.app`, signs it with the hardened runtime, packs it into an installer signed with a Developer ID Installer certificate, submits that to Apple for notarisation and staples the ticket to it.

```bash
rustup target add x86_64-apple-darwin   # once, for the universal binary

xcrun notarytool store-credentials notarytool \
  --apple-id <apple-id> --team-id <team-id> --password <app-specific-password>

scripts/macos-release.sh
```

Both certificates are found in the keychain on their own; set `GAZE_APP_CERT` and `GAZE_INSTALLER_CERT` only if you hold more than one Developer ID.

The installer lands in `dist/`. Run `scripts/macos-release.sh --help` for the rest of the settings, `--unsigned` to check the packaging without certificates, and `--host-arch` to skip the universal build.

`Gaze.app` sets `LSUIElement`, so it stays out of the Dock and the app switcher the same way it stays out of the Windows taskbar. Quit it from the right-click menu on the eyes.

The icon comes from `assets/icon.png`, drawn by `cargo run --release --example icon`. Run that again if you change the face, and the next release picks the new icon up.

Installing to `/Applications` moves the executable, so switch start-at-login off and on again afterwards to point the login item at the installed copy.

## Development checks

```sh
cargo test
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
```
