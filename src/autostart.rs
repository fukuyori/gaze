use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, path::Path};

use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    },
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Gaze";

pub fn is_enabled() -> io::Result<bool> {
    let subkey = wide(RUN_KEY);
    let value_name = wide(VALUE_NAME);
    let mut size = 0_u32;
    // SAFETY: all strings are null-terminated, and the null data pointer asks
    // Windows only for the value size and existence.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };

    match result {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        error => Err(io::Error::from_raw_os_error(error as i32)),
    }
}

pub fn set_enabled(enabled: bool) -> io::Result<()> {
    let subkey = wide(RUN_KEY);
    let value_name = wide(VALUE_NAME);

    let result = if enabled {
        let executable = std::env::current_exe()?;
        let command = wide(startup_command(&executable));
        // SAFETY: pointers reference null-terminated UTF-16 buffers for the
        // duration of the call, including the REG_SZ terminator in `cbData`.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value_name.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * size_of::<u16>()) as u32,
            )
        }
    } else {
        // SAFETY: both pointers reference null-terminated UTF-16 buffers.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), value_name.as_ptr()) }
    };

    if result == ERROR_SUCCESS || (!enabled && result == ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

fn startup_command(executable: &Path) -> String {
    format!(r#""{}""#, executable.display())
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_executable_is_quoted() {
        assert_eq!(
            startup_command(Path::new(r"C:\Program Files\Gaze\gaze.exe")),
            r#""C:\Program Files\Gaze\gaze.exe""#
        );
    }
}
