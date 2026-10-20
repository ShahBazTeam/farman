//! Shell profiles. Built-in profiles are discovered on the current machine;
//! user-defined profiles come from the versioned TOML configuration.

use std::path::PathBuf;

use crate::config::{Config, UserProfile};
use crate::session::{find_program, SpawnOptions};

#[derive(Debug, Clone)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub builtin: bool,
}

impl Profile {
    pub fn spawn_options(
        &self,
        id: u64,
        cols: u16,
        rows: u16,
        cell_width_px: u16,
        cell_height_px: u16,
        config: &Config,
    ) -> SpawnOptions {
        let cwd = self
            .cwd
            .clone()
            .or_else(|| {
                if config.startup_directory.trim().is_empty() {
                    None
                } else {
                    Some(PathBuf::from(config.startup_directory.trim()))
                }
            })
            .filter(|p| p.is_dir())
            .or_else(|| Some(crate::config::user_home()));
        SpawnOptions {
            id,
            profile_name: self.name.clone(),
            program: self.program.clone(),
            args: self.args.clone(),
            cwd,
            cols,
            rows,
            cell_width_px,
            cell_height_px,
            scrollback: config.scrollback,
            cursor_style: config.cursor.style.clone(),
            cursor_blink: config.cursor.blink,
        }
    }
}

pub fn discover(config: &Config) -> Vec<Profile> {
    let mut profiles = Vec::new();

    if let Some(path) = find_program("powershell.exe") {
        profiles.push(Profile {
            id: "powershell".to_owned(),
            name: "Windows PowerShell".to_owned(),
            program: path.to_string_lossy().into_owned(),
            // No profile scripts are executed implicitly. Users who want their
            // PowerShell profile can add a custom profile with explicit args.
            args: vec!["-NoLogo".to_owned(), "-NoProfile".to_owned()],
            cwd: None,
            builtin: true,
        });
    }

    if let Some(path) = find_pwsh() {
        profiles.push(Profile {
            id: "pwsh".to_owned(),
            name: "PowerShell 7".to_owned(),
            program: path.to_string_lossy().into_owned(),
            args: vec!["-NoLogo".to_owned(), "-NoProfile".to_owned()],
            cwd: None,
            builtin: true,
        });
    }

    if let Some(path) = find_program("cmd.exe") {
        profiles.push(Profile {
            id: "cmd".to_owned(),
            name: "Command Prompt".to_owned(),
            program: path.to_string_lossy().into_owned(),
            // /D disables cmd's AutoRun registry commands. /K would be
            // arbitrary startup execution, so it is deliberately not used.
            args: vec!["/D".to_owned()],
            cwd: None,
            builtin: true,
        });
    }

    if let Some(path) = find_program("wsl.exe") {
        profiles.push(Profile {
            id: "wsl".to_owned(),
            name: "WSL".to_owned(),
            program: path.to_string_lossy().into_owned(),
            args: Vec::new(),
            cwd: None,
            builtin: true,
        });
    }

    for (id, user) in &config.profiles {
        if let Some(profile) = from_user(id, user) {
            // A custom profile with a built-in id intentionally replaces the
            // built-in version.
            profiles.retain(|p| p.id != profile.id);
            profiles.push(profile);
        }
    }

    if profiles.is_empty() {
        // This is a last-resort fallback for unusual installations.
        profiles.push(Profile {
            id: "cmd".to_owned(),
            name: "Command Prompt".to_owned(),
            program: "cmd.exe".to_owned(),
            args: vec!["/D".to_owned()],
            cwd: None,
            builtin: true,
        });
    }
    profiles
}

fn from_user(id: &str, user: &UserProfile) -> Option<Profile> {
    if user.program.trim().is_empty() {
        return None;
    }
    let cwd = if user.cwd.trim().is_empty() {
        None
    } else {
        let path = PathBuf::from(user.cwd.trim());
        path.is_dir().then_some(path)
    };
    Some(Profile {
        id: id.to_owned(),
        name: if user.name.trim().is_empty() {
            id.to_owned()
        } else {
            user.name.clone()
        },
        program: user.program.trim().to_owned(),
        args: user.args.clone(),
        cwd,
        builtin: false,
    })
}

fn find_pwsh() -> Option<PathBuf> {
    if let Some(path) = find_program("pwsh.exe") {
        return Some(path);
    }
    let mut candidates = Vec::new();
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(program_files).join("PowerShell/7/pwsh.exe"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Microsoft/WindowsApps/pwsh.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn selected_profile<'a>(profiles: &'a [Profile], config: &Config) -> &'a Profile {
    profiles
        .iter()
        .find(|p| p.id == config.default_profile)
        .unwrap_or(&profiles[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_at_least_cmd_on_windows() {
        let config = Config::default();
        let profiles = discover(&config);
        assert!(profiles.iter().any(|p| p.id == "cmd"));
    }

    #[test]
    fn custom_profile_replaces_builtin_and_keeps_cwd() {
        let mut config = Config::default();
        let cwd = std::env::temp_dir();
        config.profiles.insert(
            "cmd".to_owned(),
            UserProfile {
                name: "My CMD".to_owned(),
                program: "cmd.exe".to_owned(),
                args: vec!["/D".to_owned()],
                cwd: cwd.to_string_lossy().into_owned(),
            },
        );
        let profiles = discover(&config);
        let cmd = profiles.iter().find(|p| p.id == "cmd").unwrap();
        assert_eq!(cmd.name, "My CMD");
        assert!(!cmd.builtin);
        assert_eq!(cmd.cwd.as_deref(), Some(cwd.as_path()));
    }
}
