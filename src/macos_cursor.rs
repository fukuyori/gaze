//! The desktop cursor position on macOS.
//!
//! egui only reports the pointer while it is over the window, which leaves the
//! distance to the cursor pinned to at most half the window diagonal — far too
//! short for the eyes to ever squint. CoreGraphics reports the cursor anywhere
//! on the desktop, the same way `GetCursorPos` does on Windows.

use std::{ffi::c_void, ptr};

use eframe::egui::Pos2;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *mut c_void);
}

/// The cursor in global display coordinates: points, with the origin at the
/// top-left of the main display. That is the same space egui reports
/// `inner_rect` and `outer_rect` in, so the two can be subtracted directly.
pub fn desktop_position() -> Option<Pos2> {
    // SAFETY: a null event source asks CoreGraphics for the current cursor
    // state. The returned event is owned by us, read once, and then released.
    unsafe {
        let event = CGEventCreate(ptr::null_mut());
        if event.is_null() {
            return None;
        }
        let location = CGEventGetLocation(event);
        CFRelease(event);
        Some(Pos2::new(location.x as f32, location.y as f32))
    }
}
