# Gaze

[日本語](README.ja.md)

An expressive xeyes-style desktop application built with egui / eframe.

## Features

- Two eyes follow the desktop mouse cursor
- The eyes squint as the cursor moves farther away
- Blinks with naturally distributed intervals from 2.25 to 12 seconds, with a 12% chance of a double blink
- After 3 minutes without mouse or keyboard input, checks for a yawn every 10 seconds with `x = idle seconds / 10` and probability `y = (x² - 324) / 576`; closes its eyes and sleeps after 5 minutes
- Transparent background with no title bar or taskbar entry
- Show, hide, and exit controls in the system tray
- Optional automatic startup when signing in to Windows
- Resizable by dragging the outer edges of the eyes
- Restores the previous window position and size at startup
- DPI scaling support

Drag near the center of either eye to move the window. Drag the left or right outer edge to resize it horizontally, or the top or bottom edge to resize it vertically. Left-click or right-click the tray icon to open the show/hide, automatic startup, and exit menu.

Automatic startup is configured only for the current Windows user and does not require administrator privileges.

On Windows, idle detection counts only actual mouse cursor movement and Raw Input key presses. Touch, pen, other HID activity, and timestamp-only input updates do not reset the idle timer.

## Running

```powershell
cargo run --release
```

On Windows, Gaze tracks the cursor even when it is outside the window. On other operating systems, it currently tracks the cursor only while it is inside the window.

## Development checks

```powershell
cargo test
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
```
