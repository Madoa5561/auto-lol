use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use windows_sys::Win32::UI::Shell::{CSIDL_LOCAL_APPDATA, SHGFP_TYPE_CURRENT, SHGetFolderPathW};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub auto_accept: bool,
    pub auto_pick: bool,
    pub auto_lock: bool,
    pub auto_ban: bool,
    pub top: Vec<String>,
    pub jungle: Vec<String>,
    pub middle: Vec<String>,
    pub bottom: Vec<String>,
    pub utility: Vec<String>,
    pub ban_top: Vec<String>,
    pub ban_jungle: Vec<String>,
    pub ban_middle: Vec<String>,
    pub ban_bottom: Vec<String>,
    pub ban_utility: Vec<String>,
}

impl Settings {
    pub fn load() -> (Self, bool) {
        let path = settings_path();
        if let Some(settings) = load_settings_from(&path) {
            return (settings, true);
        }
        if let Some(legacy_path) = redirected_settings_path()
            && legacy_path != path
            && let Some(settings) = load_settings_from(&legacy_path)
        {
            return (settings, false);
        }
        (Self::default(), false)
    }

    pub fn save(&self) -> io::Result<()> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()
    }

    pub fn picks_for_position(&self, assigned_position: &str) -> &[String] {
        match assigned_position.trim().to_ascii_lowercase().as_str() {
            "top" => &self.top,
            "jungle" => &self.jungle,
            "middle" | "mid" => &self.middle,
            "bottom" | "bot" | "adc" => &self.bottom,
            "utility" | "support" => &self.utility,
            _ => &[],
        }
    }

    pub fn bans_for_position(&self, assigned_position: &str) -> &[String] {
        match assigned_position.trim().to_ascii_lowercase().as_str() {
            "top" => &self.ban_top,
            "jungle" => &self.ban_jungle,
            "middle" | "mid" => &self.ban_middle,
            "bottom" | "bot" | "adc" => &self.ban_bottom,
            "utility" | "support" => &self.ban_utility,
            _ => &[],
        }
    }
}

#[allow(dead_code)]
pub fn parse_candidates(value: &str) -> Vec<String> {
    value
        .split([',', '、', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[allow(dead_code)]
pub fn format_candidates(values: &[String]) -> String {
    values.join(", ")
}

fn settings_path() -> PathBuf {
    app_data_dir().join("settings.json")
}

fn redirected_settings_path() -> Option<PathBuf> {
    redirected_local_app_data().map(|path| path.join("LanePilot").join("settings.json"))
}

fn load_settings_from(path: &Path) -> Option<Settings> {
    let json = fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

pub fn champion_icon_path(champion_id: i64) -> PathBuf {
    app_data_dir()
        .join("champion-icons")
        .join(format!("{champion_id}.png"))
}

fn app_data_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| {
            executable
                .parent()
                .map(|parent| parent.join("LanePilotData"))
        })
        .or_else(|| redirected_local_app_data().map(|path| path.join("LanePilot")))
        .unwrap_or_else(|| std::env::temp_dir().join("LanePilot"))
}

fn redirected_local_app_data() -> Option<PathBuf> {
    let mut path = [0u16; 260];
    let result = unsafe {
        SHGetFolderPathW(
            null_mut(),
            CSIDL_LOCAL_APPDATA as i32,
            null_mut(),
            SHGFP_TYPE_CURRENT as u32,
            path.as_mut_ptr(),
        )
    };
    if result < 0 {
        return None;
    }
    let length = path.iter().position(|character| *character == 0)?;
    Some(PathBuf::from(String::from_utf16_lossy(&path[..length])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_japanese_and_ascii_commas() {
        assert_eq!(
            parse_candidates("Ahri, Lux、 アニー\nOrianna"),
            ["Ahri", "Lux", "アニー", "Orianna"]
        );
    }

    #[test]
    fn maps_client_position_names() {
        let settings = Settings {
            bottom: vec!["Jinx".into()],
            utility: vec!["Lulu".into()],
            ban_middle: vec!["Zed".into()],
            ..Settings::default()
        };
        assert_eq!(settings.picks_for_position("BOTTOM"), ["Jinx"]);
        assert_eq!(settings.picks_for_position("support"), ["Lulu"]);
        assert_eq!(settings.bans_for_position("mid"), ["Zed"]);
    }

    #[test]
    fn json_roundtrip_preserves_all_settings() {
        let settings = Settings {
            auto_accept: true,
            auto_pick: true,
            auto_lock: true,
            auto_ban: true,
            top: vec!["Garen".into()],
            jungle: vec!["Vi".into()],
            middle: vec!["Ahri".into()],
            bottom: vec!["Jinx".into()],
            utility: vec!["Lulu".into()],
            ban_top: vec!["Darius".into()],
            ban_jungle: vec!["LeeSin".into()],
            ban_middle: vec!["Zed".into()],
            ban_bottom: vec!["Draven".into()],
            ban_utility: vec!["Nautilus".into()],
        };

        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, settings);
    }
}
