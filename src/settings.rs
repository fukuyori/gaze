use std::{fs, io, path::PathBuf};

use crate::animation::BlinkRate;

/// Opacity levels offered in the tray and context menus, in percent.
pub const OPACITY_CHOICES: [u8; 4] = [100, 75, 50, 25];

const MIN_OPACITY: u8 = 10;
const ALWAYS_ON_TOP_KEY: &str = "always_on_top";
const ALL_DESKTOPS_KEY: &str = "show_on_all_desktops";
const OPACITY_KEY: &str = "opacity";
const BLINK_KEY: &str = "blink";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub always_on_top: bool,
    /// Whether the window follows the user between desktops. Only macOS acts on
    /// this; Windows offers no supported way to pin a window across its virtual
    /// desktops.
    pub show_on_all_desktops: bool,
    pub opacity_percent: u8,
    pub blink: BlinkRate,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            always_on_top: false,
            show_on_all_desktops: true,
            opacity_percent: 100,
            blink: BlinkRate::default(),
        }
    }
}

impl Settings {
    /// Reads the stored settings, falling back to the defaults for anything
    /// missing or unreadable.
    pub fn load() -> Self {
        config_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .map_or_else(Self::default, |text| parse(&text))
    }

    pub fn save(self) -> io::Result<()> {
        let path = config_path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no settings directory for this user",
            )
        })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serialize(self))
    }

    /// Opacity as a 0..=1 factor.
    pub fn opacity(self) -> f32 {
        f32::from(self.opacity_percent.clamp(MIN_OPACITY, 100)) / 100.0
    }
}

fn config_path() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);

    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
    });

    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));

    Some(base?.join("Gaze").join("settings.conf"))
}

fn serialize(settings: Settings) -> String {
    format!(
        "{ALWAYS_ON_TOP_KEY}={}\n{ALL_DESKTOPS_KEY}={}\n{OPACITY_KEY}={}\n{BLINK_KEY}={}\n",
        settings.always_on_top,
        settings.show_on_all_desktops,
        settings.opacity_percent.clamp(MIN_OPACITY, 100),
        settings.blink.key()
    )
}

fn parse(text: &str) -> Settings {
    let mut settings = Settings::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            ALWAYS_ON_TOP_KEY => settings.always_on_top = value == "true",
            ALL_DESKTOPS_KEY => settings.show_on_all_desktops = value == "true",
            BLINK_KEY => {
                if let Some(rate) = BlinkRate::from_key(value) {
                    settings.blink = rate;
                }
            }
            OPACITY_KEY => {
                if let Ok(percent) = value.parse::<u8>() {
                    settings.opacity_percent = percent.clamp(MIN_OPACITY, 100);
                }
            }
            _ => {}
        }
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_settings_file_format() {
        let settings = Settings {
            always_on_top: true,
            show_on_all_desktops: false,
            opacity_percent: 50,
            blink: BlinkRate::Slow,
        };
        assert_eq!(parse(&serialize(settings)), settings);
    }

    #[test]
    fn a_file_written_before_desktops_were_a_setting_still_loads() {
        let settings = parse("always_on_top=true\nopacity=50\n");
        assert!(settings.always_on_top);
        assert_eq!(settings.opacity_percent, 50);
        assert!(
            settings.show_on_all_desktops,
            "following the user between desktops is the default"
        );
        assert_eq!(settings.blink, BlinkRate::Standard);
    }

    #[test]
    fn an_unknown_blink_rate_falls_back_to_the_standard_one() {
        assert_eq!(parse("blink=glacial\n").blink, BlinkRate::Standard);
        assert_eq!(parse("blink=off\n").blink, BlinkRate::Off);
    }

    #[test]
    fn unknown_or_malformed_lines_keep_the_defaults() {
        assert_eq!(
            parse("# comment\nwindow_level\nopacity=not-a-number\n"),
            Settings::default()
        );
    }

    #[test]
    fn opacity_stays_within_a_visible_range() {
        assert_eq!(parse("opacity=0").opacity_percent, MIN_OPACITY);
        assert_eq!(parse("opacity=200").opacity_percent, 100);
        assert!((Settings::default().opacity() - 1.0).abs() < f32::EPSILON);
    }
}
