use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    NewTerminal,
    MoveWindow(Direction),
    KillWindow,
    Quit,
    /// Jumps directly to workspace N (1-9), bindable to any chord --
    /// separate from (and in addition to) the `leader` + digit sequence.
    SwitchWorkspace(usize),
    /// Toggles resize mode: while active, arrow keys grow/shrink the
    /// focused window; every other key is ignored until `resize_exit` is
    /// pressed.
    ToggleResizeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: KeyModifiers,
    pub code: KeyCode,
}

impl Chord {
    pub fn new(mods: KeyModifiers, code: KeyCode) -> Self {
        Self { mods, code }
    }
}

/// Keybindings for the window manager itself. Anything not matched here is
/// forwarded to the focused terminal as raw input.
///
/// Two patterns are handled outside of the generic `binds` map because they
/// are dynamic (parameterized by a number) rather than fixed actions:
///   - Alt+<digit 1-9>                    -> focus window N in the current workspace
///   - `leader` then Alt+<digit 1-9>      -> switch to workspace N
///
/// Both require Alt rather than a bare digit: a bare, unmodified digit would
/// swallow every number typed in a shell. `leader` must likewise carry a
/// modifier (default: Alt+W) rather than a bare key, for the same reason.
/// Alt (not Ctrl) is used for the digit half because most terminals only
/// send a distinguishable Ctrl+<modifier> event for digits 4-7 --
/// Ctrl+1/2/3/8/9 either collide with other keys (Esc, Backspace) or arrive
/// with no modifier bit at all. Alt+<digit> is the classic ESC-prefixed
/// "meta key" encoding and works reliably for all of 1-9 across virtually
/// every terminal.
pub struct Keybinds {
    pub binds: HashMap<Chord, Action>,
    pub leader: Chord,
    /// The only chord that exits resize mode. Kept separate from
    /// `ToggleResizeMode` itself: resize mode is strictly modal, so
    /// re-pressing the chord that opened it does nothing rather than
    /// closing it.
    pub resize_exit: Chord,
}

impl Keybinds {
    pub fn defaults() -> Self {
        let mut binds = HashMap::new();
        binds.insert(
            Chord::new(KeyModifiers::ALT, KeyCode::Char('t')),
            Action::NewTerminal,
        );
        binds.insert(
            Chord::new(KeyModifiers::ALT, KeyCode::Up),
            Action::MoveWindow(Direction::Up),
        );
        binds.insert(
            Chord::new(KeyModifiers::ALT, KeyCode::Down),
            Action::MoveWindow(Direction::Down),
        );
        binds.insert(
            Chord::new(KeyModifiers::ALT, KeyCode::Left),
            Action::MoveWindow(Direction::Left),
        );
        binds.insert(
            Chord::new(KeyModifiers::ALT, KeyCode::Right),
            Action::MoveWindow(Direction::Right),
        );
        binds.insert(
            Chord::new(KeyModifiers::ALT, KeyCode::Char('q')),
            Action::Quit,
        );
        binds.insert(
            Chord::new(KeyModifiers::ALT, KeyCode::Char('x')),
            Action::KillWindow,
        );
        binds.insert(
            Chord::new(KeyModifiers::ALT, KeyCode::Char('r')),
            Action::ToggleResizeMode,
        );

        Self {
            binds,
            leader: Chord::new(KeyModifiers::ALT, KeyCode::Char('w')),
            resize_exit: Chord::new(KeyModifiers::NONE, KeyCode::Esc),
        }
    }
}
