# Changelog

All notable changes to Gaze are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Gaze uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0]

The release that makes Gaze a macOS application rather than a Windows one that
happens to compile elsewhere.

### Added

- macOS support for everything Gaze already did on Windows: resizing by the
  edges of the eyes, automatic startup, and settings that survive a restart.
- A settings menu on the right-click of either eye, for the platforms that have
  no tray icon.
- An always-on-top setting, on both platforms.
- An opacity setting between 100% and 25%. macOS fades the painted face;
  Windows fades the window itself, and only takes the layered-window path while
  the setting is below 100%, so a fully opaque Gaze renders exactly as before.
- A setting for how much Gaze blinks: standard, slow, or off.
- A setting to show Gaze on every desktop, on by default. Windows has no
  supported API for pinning a window across its virtual desktops, so the tray
  menu does not offer it.
- Start at login on macOS, through a LaunchAgent in the user's own
  `~/Library/LaunchAgents`.
- A quit item in the right-click menu, the only way out of an application that
  keeps itself out of the Dock.
- An application icon, drawn by `cargo run --release --example icon`.
- `scripts/macos-release.sh`, which builds a universal binary, wraps it in
  `Gaze.app`, signs it, packs it into an installer, and submits that to Apple
  for notarisation.

### Changed

- The eyes follow the cursor through a critically damped spring rather than
  snapping to it, so a small movement is tracked with a lazy pursuit and a
  large one with a fast saccade.
- A full squint now arrives at half the monitor diagonal instead of at nearly
  three quarters of it, which no cursor on a single display could reach.
- The iris is sized from both eye axes rather than from the width alone, so
  narrowing the window shrinks it exactly as much as flattening it does.
- Blinking is calmer: intervals run from 3 to 13 seconds rather than 2.25 to 12
  (about nine blinks a minute, down from eleven), and a double blink follows
  one blink in sixteen rather than one in eight.
- Every couple of minutes one blink is now a slow, deliberate close lasting
  almost a second.
- The window can be dragged down to 120x80 points, from 260x150. The face is
  laid out proportionally, so a small window shows a small face rather than one
  cropped by the edges.
- The band that resizes rather than drags scales with the face instead of being
  a fixed ten points, and the egui and Win32 sides of it now agree.
- Settings live in `settings.conf` under the user's configuration directory.

### Fixed

- The iris could be painted outside the eye. It is trimmed to the eyelid's own
  curve now, rather than to a rectangle between the lids, and stops at the
  inner edge of the outline so that a translucent Gaze does not show it through
  the lid.
- The settings menu was cut off at the bottom of a window shorter than itself.
- A build warning about an unnecessary `mut`.

## [0.1.5]

### Fixed

- Stalled eye rendering recovers on its own, through native Windows repaint
  notifications.

## [0.1.4]

### Changed

- Blink intervals follow a natural distribution from 2.25 to 12 seconds, with a
  12% chance of a double blink.
- Yawning waits for three minutes of idleness and is then checked every ten
  seconds against `y = (x² - 324) / 576`, where `x` is the idle time in units of
  ten seconds; sleep follows after five minutes.

### Added

- Idle detection on Windows counts only real cursor movement and Raw Input key
  presses, so touch, pen and other HID activity no longer keep Gaze awake.

## [0.1.3]

### Added

- Gaze yawns once mouse and keyboard input has been idle, then closes its eyes
  and sleeps.

## [0.1.2]

### Changed

- Blinking and pupil movement reworked.

## [0.1.1]

### Fixed

- Eye rendering recovers after the window stops being repainted.

## [0.1.0]

The first release: two eyes following the desktop cursor in a transparent,
borderless window, with a system tray icon and optional startup with Windows.
