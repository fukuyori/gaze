use std::{
    io,
    sync::atomic::{AtomicBool, AtomicI32, Ordering},
};

use eframe::egui::Rect;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::ScreenToClient,
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{GetCursorPos, HTBOTTOM, HTLEFT, HTRIGHT, HTTOP, WM_NCHITTEST},
    },
};

const SUBCLASS_ID: usize = 0x475A;

struct HitState {
    left: AtomicI32,
    top: AtomicI32,
    right: AtomicI32,
    bottom: AtomicI32,
    edge: AtomicI32,
    enabled: AtomicBool,
}

impl HitState {
    fn new() -> Self {
        Self {
            left: AtomicI32::new(0),
            top: AtomicI32::new(0),
            right: AtomicI32::new(0),
            bottom: AtomicI32::new(0),
            edge: AtomicI32::new(1),
            enabled: AtomicBool::new(false),
        }
    }

    fn direction_at(&self, x: i32, y: i32) -> Option<u32> {
        if !self.enabled.load(Ordering::Acquire) {
            return None;
        }

        let left = (x - self.left.load(Ordering::Relaxed)).abs();
        let right = (x - self.right.load(Ordering::Relaxed)).abs();
        let top = (y - self.top.load(Ordering::Relaxed)).abs();
        let bottom = (y - self.bottom.load(Ordering::Relaxed)).abs();
        let edge = self.edge.load(Ordering::Relaxed);
        let horizontal = left.min(right);
        let vertical = top.min(bottom);

        if horizontal < edge && horizontal <= vertical {
            Some(if left <= right { HTLEFT } else { HTRIGHT })
        } else if vertical < edge {
            Some(if top <= bottom { HTTOP } else { HTBOTTOM })
        } else {
            None
        }
    }
}

pub struct NativeResize {
    window: HWND,
    state: Box<HitState>,
}

impl NativeResize {
    pub fn new(window: isize) -> io::Result<Self> {
        let state = Box::new(HitState::new());
        // SAFETY: the boxed state has a stable address and is kept alive until
        // the subclass is removed in Drop.
        let installed = unsafe {
            SetWindowSubclass(
                window as HWND,
                Some(resize_subclass_proc),
                SUBCLASS_ID,
                (&*state as *const HitState) as usize,
            )
        };
        if installed == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            window: window as HWND,
            state,
        })
    }

    pub fn update(&self, bounds: Rect, pixels_per_point: f32, enabled: bool) {
        self.state.enabled.store(false, Ordering::Release);
        self.state.left.store(
            (bounds.left() * pixels_per_point).round() as i32,
            Ordering::Relaxed,
        );
        self.state.top.store(
            (bounds.top() * pixels_per_point).round() as i32,
            Ordering::Relaxed,
        );
        self.state.right.store(
            (bounds.right() * pixels_per_point).round() as i32,
            Ordering::Relaxed,
        );
        self.state.bottom.store(
            (bounds.bottom() * pixels_per_point).round() as i32,
            Ordering::Relaxed,
        );
        self.state.edge.store(
            (12.0 * pixels_per_point).round().max(1.0) as i32,
            Ordering::Relaxed,
        );
        self.state.enabled.store(enabled, Ordering::Release);
    }
}

impl Drop for NativeResize {
    fn drop(&mut self) {
        self.state.enabled.store(false, Ordering::Release);
        // SAFETY: this removes the exact callback installed by `new` before
        // the callback's state is dropped.
        unsafe {
            RemoveWindowSubclass(self.window, Some(resize_subclass_proc), SUBCLASS_ID);
        }
    }
}

unsafe extern "system" fn resize_subclass_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    state: usize,
) -> LRESULT {
    if message == WM_NCHITTEST {
        // SAFETY: `state` points to the boxed HitState owned by NativeResize,
        // which removes this callback before freeing it.
        let state = unsafe { &*(state as *const HitState) };
        let mut cursor = POINT { x: 0, y: 0 };
        // SAFETY: `cursor` is writable and `window` is the live subclassed HWND.
        if unsafe { GetCursorPos(&mut cursor) } != 0
            && unsafe { ScreenToClient(window, &mut cursor) } != 0
            && let Some(direction) = state.direction_at(cursor.x, cursor.y)
        {
            return direction as LRESULT;
        }
    }

    // SAFETY: forwards all messages not handled above to the next subclass.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> HitState {
        let state = HitState::new();
        state.left.store(10, Ordering::Relaxed);
        state.top.store(30, Ordering::Relaxed);
        state.right.store(370, Ordering::Relaxed);
        state.bottom.store(190, Ordering::Relaxed);
        state.edge.store(12, Ordering::Relaxed);
        state.enabled.store(true, Ordering::Release);
        state
    }

    #[test]
    fn outer_eye_edges_are_native_resize_borders() {
        let state = state();
        assert_eq!(state.direction_at(14, 110), Some(HTLEFT));
        assert_eq!(state.direction_at(366, 110), Some(HTRIGHT));
        assert_eq!(state.direction_at(100, 34), Some(HTTOP));
        assert_eq!(state.direction_at(280, 186), Some(HTBOTTOM));
    }

    #[test]
    fn eye_interior_is_not_a_resize_border() {
        assert_eq!(state().direction_at(280, 110), None);
    }
}
