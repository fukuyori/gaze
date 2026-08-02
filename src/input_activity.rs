use std::{
    io,
    mem::size_of,
    sync::atomic::{AtomicU32, Ordering},
};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::{
            GetRawInputData, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK,
            RIDEV_REMOVE, RIM_TYPEKEYBOARD, RegisterRawInputDevices,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{RI_KEY_BREAK, WM_INPUT},
    },
};

const SUBCLASS_ID: usize = 0x475B;
const HID_USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x01;
const HID_USAGE_GENERIC_KEYBOARD: u16 = 0x06;

/// Counts physical and virtual keyboard make events received through Raw Input.
pub struct KeyboardActivity {
    window: HWND,
    marker: Box<AtomicU32>,
}

impl KeyboardActivity {
    pub fn new(window: isize) -> io::Result<Self> {
        let window = window as HWND;
        let marker = Box::new(AtomicU32::new(0));
        // SAFETY: the boxed marker has a stable address and is kept alive until
        // this exact subclass is removed in Drop.
        let installed = unsafe {
            SetWindowSubclass(
                window,
                Some(keyboard_subclass_proc),
                SUBCLASS_ID,
                (&*marker as *const AtomicU32) as usize,
            )
        };
        if installed == 0 {
            return Err(io::Error::last_os_error());
        }

        let keyboard = RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC_DESKTOP,
            usUsage: HID_USAGE_GENERIC_KEYBOARD,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: window,
        };
        // SAFETY: `keyboard` describes one valid Raw Input registration whose
        // target is the live application window.
        let registered =
            unsafe { RegisterRawInputDevices(&keyboard, 1, size_of::<RAWINPUTDEVICE>() as u32) };
        if registered == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: removes the subclass installed above before its state is
            // returned to the caller or dropped.
            unsafe {
                RemoveWindowSubclass(window, Some(keyboard_subclass_proc), SUBCLASS_ID);
            }
            return Err(error);
        }

        Ok(Self { window, marker })
    }

    pub fn marker(&self) -> u32 {
        self.marker.load(Ordering::Acquire)
    }
}

impl Drop for KeyboardActivity {
    fn drop(&mut self) {
        let remove = RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC_DESKTOP,
            usUsage: HID_USAGE_GENERIC_KEYBOARD,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: std::ptr::null_mut(),
        };
        // SAFETY: unregisters the same keyboard usage registered by new. Raw
        // Input requires a null target when RIDEV_REMOVE is used.
        unsafe {
            RegisterRawInputDevices(&remove, 1, size_of::<RAWINPUTDEVICE>() as u32);
            RemoveWindowSubclass(self.window, Some(keyboard_subclass_proc), SUBCLASS_ID);
        }
    }
}

unsafe extern "system" fn keyboard_subclass_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    state: usize,
) -> LRESULT {
    if message == WM_INPUT {
        let mut raw = RAWINPUT::default();
        let mut raw_size = size_of::<RAWINPUT>() as u32;
        // SAFETY: `raw` is a writable buffer of `raw_size`; `lparam` is the
        // HRAWINPUT supplied with this WM_INPUT message.
        let copied = unsafe {
            GetRawInputData(
                lparam as _,
                RID_INPUT,
                (&mut raw as *mut RAWINPUT).cast(),
                &mut raw_size,
                size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        if copied != u32::MAX && raw.header.dwType == RIM_TYPEKEYBOARD {
            // SAFETY: the header identifies the active RAWINPUT union member
            // as keyboard data.
            let keyboard = unsafe { raw.data.keyboard };
            if is_keyboard_press(keyboard.Flags, keyboard.VKey) {
                // SAFETY: state points to the boxed atomic owned by
                // KeyboardActivity until this subclass is removed.
                unsafe { &*(state as *const AtomicU32) }.fetch_add(1, Ordering::Release);
            }
        }
    }

    // SAFETY: forwards every message, including WM_INPUT cleanup, to the next
    // subclass in the chain.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

fn is_keyboard_press(flags: u16, virtual_key: u16) -> bool {
    flags & RI_KEY_BREAK as u16 == 0 && virtual_key != 255
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_make_and_repeat_are_activity() {
        assert!(is_keyboard_press(0, b'A' as u16));
    }

    #[test]
    fn key_release_and_fake_key_are_not_activity() {
        assert!(!is_keyboard_press(RI_KEY_BREAK as u16, b'A' as u16));
        assert!(!is_keyboard_press(0, 255));
    }
}
