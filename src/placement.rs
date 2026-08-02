use std::{
    ffi::OsStr,
    io,
    os::windows::ffi::OsStrExt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW},
};

const SETTINGS_KEY: &str = r"Software\Gaze";
const PLACEMENT_VALUE: &str = "WindowPlacement";
const SAVE_DELAY: Duration = Duration::from_millis(700);

pub const DEFAULT_WIDTH: f32 = 380.0;
pub const DEFAULT_HEIGHT: f32 = 220.0;
pub const MIN_WIDTH: f32 = 260.0;
pub const MIN_HEIGHT: f32 = 150.0;
pub const MAX_WIDTH: f32 = 540.0;
pub const MAX_HEIGHT: f32 = 320.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowPlacement {
    pub fn load() -> io::Result<Option<Self>> {
        let subkey = wide(SETTINGS_KEY);
        let value_name = wide(PLACEMENT_VALUE);
        let mut byte_count = 0_u32;

        // SAFETY: the strings are null-terminated. A null data pointer asks
        // Windows for the required REG_SZ buffer size only.
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut byte_count,
            )
        };
        match result {
            ERROR_FILE_NOT_FOUND => return Ok(None),
            ERROR_SUCCESS => {}
            error => return Err(io::Error::from_raw_os_error(error as i32)),
        }

        let mut buffer = vec![0_u16; (byte_count as usize).div_ceil(size_of::<u16>())];
        // SAFETY: `buffer` has the byte size reported by the first call and
        // remains live and writable for the duration of this call.
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut byte_count,
            )
        };
        if result != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(result as i32));
        }

        let length = buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(buffer.len());
        let value = String::from_utf16(&buffer[..length])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(parse(&value))
    }

    pub fn save(self) -> io::Result<()> {
        let subkey = wide(SETTINGS_KEY);
        let value_name = wide(PLACEMENT_VALUE);
        let data = wide(format!(
            "{},{},{},{}",
            self.x, self.y, self.width, self.height
        ));

        // SAFETY: all pointers reference live UTF-16 buffers, and `data`
        // includes the terminating null required by REG_SZ.
        let result = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value_name.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * size_of::<u16>()) as u32,
            )
        };
        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(result as i32))
        }
    }

    pub fn restored_size(self) -> [f32; 2] {
        [
            (self.width as f32).clamp(MIN_WIDTH, MAX_WIDTH),
            (self.height as f32).clamp(MIN_HEIGHT, MAX_HEIGHT),
        ]
    }
}

pub struct PlacementTracker {
    latest: Option<WindowPlacement>,
    saved: Option<WindowPlacement>,
    changed_at: Option<Instant>,
    shared: SharedPlacement,
}

impl PlacementTracker {
    pub fn new(saved: Option<WindowPlacement>) -> Self {
        Self {
            latest: saved,
            saved,
            changed_at: None,
            shared: SharedPlacement::new(saved),
        }
    }

    pub fn shared(&self) -> SharedPlacement {
        self.shared.clone()
    }

    pub fn observe(&mut self, placement: WindowPlacement) {
        let now = Instant::now();
        if self.latest != Some(placement) {
            self.latest = Some(placement);
            self.shared.update(placement);
            self.changed_at = Some(now);
            return;
        }

        if self.saved != self.latest
            && self
                .changed_at
                .is_some_and(|changed_at| now.duration_since(changed_at) >= SAVE_DELAY)
            && placement.save().is_ok()
        {
            self.saved = Some(placement);
            self.changed_at = None;
        }
    }

    pub fn flush(&mut self) {
        if self.saved != self.latest
            && let Some(placement) = self.latest
            && placement.save().is_ok()
        {
            self.saved = Some(placement);
            self.changed_at = None;
        }
    }
}

#[derive(Clone)]
pub struct SharedPlacement {
    inner: Arc<SharedPlacementInner>,
}

struct SharedPlacementInner {
    sequence: AtomicU32,
    x: AtomicI32,
    y: AtomicI32,
    width: AtomicU32,
    height: AtomicU32,
    valid: AtomicBool,
}

impl SharedPlacement {
    fn new(initial: Option<WindowPlacement>) -> Self {
        let shared = Self {
            inner: Arc::new(SharedPlacementInner {
                sequence: AtomicU32::new(0),
                x: AtomicI32::new(0),
                y: AtomicI32::new(0),
                width: AtomicU32::new(0),
                height: AtomicU32::new(0),
                valid: AtomicBool::new(false),
            }),
        };
        if let Some(initial) = initial {
            shared.update(initial);
        }
        shared
    }

    fn update(&self, placement: WindowPlacement) {
        self.inner.sequence.fetch_add(1, Ordering::AcqRel);
        self.inner.x.store(placement.x, Ordering::Relaxed);
        self.inner.y.store(placement.y, Ordering::Relaxed);
        self.inner.width.store(placement.width, Ordering::Relaxed);
        self.inner.height.store(placement.height, Ordering::Relaxed);
        self.inner.valid.store(true, Ordering::Relaxed);
        self.inner.sequence.fetch_add(1, Ordering::Release);
    }

    fn latest(&self) -> Option<WindowPlacement> {
        loop {
            let before = self.inner.sequence.load(Ordering::Acquire);
            if !before.is_multiple_of(2) {
                std::hint::spin_loop();
                continue;
            }
            let placement = WindowPlacement {
                x: self.inner.x.load(Ordering::Relaxed),
                y: self.inner.y.load(Ordering::Relaxed),
                width: self.inner.width.load(Ordering::Relaxed),
                height: self.inner.height.load(Ordering::Relaxed),
            };
            let valid = self.inner.valid.load(Ordering::Relaxed);
            let after = self.inner.sequence.load(Ordering::Acquire);
            if before == after {
                return valid.then_some(placement);
            }
        }
    }

    pub fn save_latest(&self) -> io::Result<()> {
        if let Some(placement) = self.latest() {
            placement.save()
        } else {
            Ok(())
        }
    }
}

fn parse(value: &str) -> Option<WindowPlacement> {
    let mut fields = value.split(',');
    let placement = WindowPlacement {
        x: fields.next()?.parse().ok()?,
        y: fields.next()?.parse().ok()?,
        width: fields.next()?.parse().ok()?,
        height: fields.next()?.parse().ok()?,
    };
    fields.next().is_none().then_some(placement)
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_position_on_a_monitor_left_of_the_primary_display() {
        assert_eq!(
            parse("-1280,240,420,240"),
            Some(WindowPlacement {
                x: -1280,
                y: 240,
                width: 420,
                height: 240,
            })
        );
    }

    #[test]
    fn rejects_incomplete_or_extra_fields() {
        assert_eq!(parse("10,20,380"), None);
        assert_eq!(parse("10,20,380,220,1"), None);
        assert_eq!(parse("x,20,380,220"), None);
    }

    #[test]
    fn restored_size_obeys_window_constraints() {
        let placement = WindowPlacement {
            x: 0,
            y: 0,
            width: 100,
            height: 900,
        };
        assert_eq!(placement.restored_size(), [MIN_WIDTH, MAX_HEIGHT]);
    }

    #[test]
    fn shared_placement_exposes_the_latest_complete_value() {
        let first = WindowPlacement {
            x: 10,
            y: 20,
            width: 380,
            height: 220,
        };
        let second = WindowPlacement {
            x: -900,
            y: 40,
            width: 430,
            height: 260,
        };
        let shared = SharedPlacement::new(Some(first));
        assert_eq!(shared.latest(), Some(first));
        shared.update(second);
        assert_eq!(shared.latest(), Some(second));
    }
}
