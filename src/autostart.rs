//! Launching Gaze automatically when the user signs in.
//!
//! Windows uses the per-user `Run` registry key and macOS a LaunchAgent under
//! the user's own `~/Library/LaunchAgents`. Neither needs administrator rights,
//! and neither touches anything outside the signed-in user's account.

#[cfg(target_os = "macos")]
pub use launch_agent::{is_enabled, set_enabled};
#[cfg(windows)]
pub use registry::{is_enabled, set_enabled};

#[cfg(windows)]
mod registry {
    use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, path::Path};

    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
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
}

#[cfg(target_os = "macos")]
mod launch_agent {
    use std::{
        fs, io,
        path::{Path, PathBuf},
    };

    /// Matches the bundle identifier the installer script gives `Gaze.app`.
    const LABEL: &str = "org.spumoni.gaze";

    pub fn is_enabled() -> io::Result<bool> {
        Ok(agent_path()?.is_file())
    }

    /// Writing the agent is enough: `launchd` reads `~/Library/LaunchAgents`
    /// when the user signs in. It is deliberately not loaded here, which would
    /// start a second Gaze on the spot.
    pub fn set_enabled(enabled: bool) -> io::Result<()> {
        let path = agent_path()?;
        if !enabled {
            return match fs::remove_file(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            };
        }

        let executable = std::env::current_exe()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, agent_plist(&executable))
    }

    fn agent_path() -> io::Result<PathBuf> {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }

    fn agent_plist(executable: &Path) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
</dict>
</plist>
"#,
            escape_xml(&executable.to_string_lossy())
        )
    }

    fn escape_xml(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_agent_runs_the_current_executable_at_login() {
            let plist = agent_plist(Path::new("/Users/someone/bin/gaze"));
            assert!(plist.contains("<string>/Users/someone/bin/gaze</string>"));
            assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
            assert!(plist.contains(LABEL));
        }

        #[test]
        fn a_path_with_xml_characters_cannot_break_the_agent() {
            let plist = agent_plist(Path::new("/Users/a&b/<gaze>"));
            assert!(plist.contains("<string>/Users/a&amp;b/&lt;gaze&gt;</string>"));
        }

        #[test]
        fn the_agent_lives_in_the_users_own_launch_agents_folder() {
            let path = agent_path().expect("HOME is set while testing");
            assert!(
                path.ends_with("Library/LaunchAgents/org.spumoni.gaze.plist"),
                "{path:?}"
            );
        }
    }
}
