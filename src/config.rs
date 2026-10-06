use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::Color;

use crate::keybinds::{Action, Chord, Direction, Keybinds};
use crate::layout::LayoutMode;

/// The config shipped as `cliwm.conf.example`, written out to
/// `~/.config/cliwm.conf` the first time CLIWM runs and that path doesn't
/// exist yet, so there's always a starting point to edit.
const DEFAULT_CONFIG: &str = include_str!("../cliwm.conf.example");

pub struct AppConfig {
    pub keybinds: Keybinds,
    pub accent: Color,
    /// A shell command CLIWM runs periodically and shows in the status row
    /// (left-aligned, next to the version label). This is the hook for
    /// building a custom status bar: CLIWM doesn't ship one, but anything
    /// that prints a line of text -- a script polling battery/CPU/time, a
    /// workspace indicator, etc. -- can be wired in here.
    pub statusbar_command: Option<String>,
    pub layout_mode: LayoutMode,
}

/// Parses `~/.config/cliwm.conf`, written in a Hyprland-flavored config
/// format:
///
/// ```text
/// # comment
/// leader = alt, w
/// accent = #61afef
/// statusbar = ~/.config/cliwm-statusbar.sh
/// bind = ctrl, t, new_terminal
/// bind = ctrl, up, move_window, up
/// bind = ctrl, q, quit
/// bind = ctrl, w, kill_window
/// bind = alt, f1, switch_workspace, 1
/// ```
///
/// Unknown or malformed lines are ignored. If the file doesn't exist yet, it
/// is created from [`DEFAULT_CONFIG`] and the built-in defaults are used.
pub fn load() -> AppConfig {
    let mut keybinds = Keybinds::defaults();
    let mut accent = Color::Cyan;
    let mut statusbar_command = None;
    let mut layout_mode = LayoutMode::OneSide;

    let Some(path) = config_path() else {
        return AppConfig {
            keybinds,
            accent,
            statusbar_command,
            layout_mode,
        };
    };

    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&path, DEFAULT_CONFIG);
    }

    let Ok(contents) = fs::read_to_string(&path) else {
        return AppConfig {
            keybinds,
            accent,
            statusbar_command,
            layout_mode,
        };
    };

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        match key {
            "leader" => {
                if let Some(chord) = parse_key(value) {
                    keybinds.leader = chord;
                }
            }
            "resize_exit" => {
                if let Some(chord) = parse_key(value) {
                    keybinds.resize_exit = chord;
                }
            }
            "accent" => {
                if let Some(color) = parse_hex_color(value) {
                    accent = color;
                }
            }
            "statusbar" => {
                if !value.is_empty() {
                    statusbar_command = Some(value.to_string());
                }
            }
            "layout" => {
                layout_mode = match value.to_lowercase().as_str() {
                    "one-side" | "one_side" | "oneside" => LayoutMode::OneSide,
                    "grid" => LayoutMode::Grid,
                    "divide" => LayoutMode::Divide,
                    _ => layout_mode,
                };
            }
            "bind" => {
                let parts: Vec<&str> = value.split(',').map(|p| p.trim()).collect();
                if parts.len() < 3 {
                    continue;
                }
                let Some(mods) = parse_mods(parts[0]) else {
                    continue;
                };
                let Some(key_code) = parse_keycode(parts[1]) else {
                    continue;
                };
                let Some(action) = parse_action(parts[2], parts.get(3).copied()) else {
                    continue;
                };
                keybinds.binds.insert(Chord::new(mods, key_code), action);
            }
            _ => {}
        }
    }

    AppConfig {
        keybinds,
        accent,
        statusbar_command,
        layout_mode,
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("cliwm.conf"))
}

fn parse_hex_color(field: &str) -> Option<Color> {
    let hex = field.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn parse_mods(field: &str) -> Option<KeyModifiers> {
    let mut mods = KeyModifiers::NONE;
    for part in field
        .split(['+', ' '])
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
    {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "alt" => mods |= KeyModifiers::ALT,
            "shift" => mods |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }
    Some(mods)
}

fn parse_keycode(field: &str) -> Option<KeyCode> {
    let lower = field.to_lowercase();
    let code = match lower.as_str() {
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        _ if lower.chars().count() == 1 => KeyCode::Char(lower.chars().next().unwrap()),
        _ if lower.starts_with('f') && lower[1..].parse::<u8>().is_ok() => {
            KeyCode::F(lower[1..].parse().unwrap())
        }
        _ => return None,
    };
    Some(code)
}

fn parse_key(field: &str) -> Option<Chord> {
    let parts: Vec<&str> = field.split(',').map(|p| p.trim()).collect();
    if parts.len() == 2 {
        let mods = parse_mods(parts[0])?;
        let code = parse_keycode(parts[1])?;
        return Some(Chord::new(mods, code));
    }
    let code = parse_keycode(field)?;
    Some(Chord::new(KeyModifiers::NONE, code))
}

fn parse_action(name: &str, arg: Option<&str>) -> Option<Action> {
    match name {
        "new_terminal" => Some(Action::NewTerminal),
        "kill_window" => Some(Action::KillWindow),
        "quit" => Some(Action::Quit),
        "toggle_resize_mode" => Some(Action::ToggleResizeMode),
        "move_window" => {
            let dir = match arg?.to_lowercase().as_str() {
                "up" => Direction::Up,
                "down" => Direction::Down,
                "left" => Direction::Left,
                "right" => Direction::Right,
                _ => return None,
            };
            Some(Action::MoveWindow(dir))
        }
        "switch_workspace" => {
            let n: usize = arg?.parse().ok()?;
            (1..=9).contains(&n).then_some(Action::SwitchWorkspace(n))
        }
        _ => None,
    }
}
