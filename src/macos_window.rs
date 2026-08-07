//! Window behaviour on macOS that neither winit nor egui exposes.
//!
//! Whether a window follows the user between Spaces is decided by the
//! `NSWindow` collection behaviour, and there is no winit API for it, so the
//! message is sent to the window through the Objective-C runtime directly.

use std::ffi::{CString, c_char, c_void};

// NSWindowCollectionBehavior.
const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
const FULL_SCREEN_AUXILIARY: usize = 1 << 8;
const DEFAULT: usize = 0;

#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    fn sel_registerName(name: *const c_char) -> *const c_void;
    fn objc_msgSend();
}

/// Shows the window on every desktop, or only on the one it was opened on.
///
/// `ns_view` is the AppKit view eframe reports for the window; a zero handle
/// means the window could not be identified and the behaviour is left alone.
pub fn set_on_all_desktops(ns_view: usize, enabled: bool) {
    if ns_view == 0 {
        return;
    }

    let behavior = if enabled {
        // Joining every Space also covers the Space a full-screen app owns,
        // which is where an auxiliary window would otherwise be hidden.
        CAN_JOIN_ALL_SPACES | FULL_SCREEN_AUXILIARY
    } else {
        DEFAULT
    };

    // SAFETY: `ns_view` is an AppKit view owned by the window eframe created,
    // and both selectors are sent with the signature AppKit declares for them.
    unsafe {
        let window_of: unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let window = window_of(ns_view as *mut c_void, selector("window"));
        if window.is_null() {
            return;
        }

        let set_collection_behavior: unsafe extern "C" fn(*mut c_void, *const c_void, usize) =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        set_collection_behavior(window, selector("setCollectionBehavior:"), behavior);
    }
}

/// # Safety
/// The caller must only pass selector names Objective-C accepts.
unsafe fn selector(name: &str) -> *const c_void {
    let name = CString::new(name).expect("selector names hold no interior nul");
    // SAFETY: the name is a live null-terminated string for this call, and the
    // returned selector is interned by the runtime rather than borrowed from it.
    unsafe { sel_registerName(name.as_ptr()) }
}
