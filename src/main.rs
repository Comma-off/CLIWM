mod app;
mod config;
mod keybinds;
mod layout;
mod pty_widget;
mod pty_window;

use std::io;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

use app::App;
use keybinds::{Action, Chord, Direction, Keybinds};

const VERSION_LABEL: &str = "CLIWM v0.1";

/// Launches `command` once as a long-lived background process -- not tied
/// to any workspace or pane, and not run through a pty -- and streams its
/// stdout back a line at a time. This is CLIWM's hook for a third-party
/// status bar: it doesn't ship one, but any persistent program that prints
/// a new line whenever its status changes (a battery/CPU monitor, a clock,
/// a workspace indicator, in the style of an i3status/waybar module) can
/// be wired in via the `statusbar` config option. CLIWM always displays
/// whatever line it printed last; the program decides its own update
/// cadence, CLIWM never re-invokes or polls it.
///
/// stdin/stderr are discarded rather than inherited, so the program can't
/// accidentally read CLIWM's own raw-mode keyboard input or write into its
/// alternate-screen rendering.
fn spawn_statusbar(command: String) -> Option<(Receiver<String>, std::process::Child)> {
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    Some((rx, child))
}

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Terminals should never report a zero-sized window, but some
/// multiplexers/sandboxes do transiently; fall back to a sane minimum so the
/// vt100 grid never has to deal with a zero dimension.
fn terminal_size(terminal: &Terminal<CrosstermBackend<io::Stdout>>) -> Result<(u16, u16)> {
    let size = terminal.size()?;
    Ok((size.height.max(24), size.width.max(80)))
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let config::AppConfig {
        keybinds,
        accent,
        statusbar_command,
        layout_mode,
    } = config::load();
    let mut app = App::new(layout_mode);

    let mut statusbar_child = None;
    let statusbar_rx = statusbar_command
        .and_then(spawn_statusbar)
        .map(|(rx, child)| {
            statusbar_child = Some(child);
            rx
        });
    let mut statusbar_text = String::new();

    let resize_hint = format!(
        "Resize mode | {} to exit",
        format_chord(&keybinds.resize_exit)
    );

    let (rows, cols) = terminal_size(terminal)?;
    let initial_area = Rect::new(0, 0, cols, rows.saturating_sub(1).max(1));
    app.new_terminal(initial_area)?;

    loop {
        if let Some(rx) = &statusbar_rx {
            while let Ok(text) = rx.try_recv() {
                statusbar_text = text;
            }
        }

        for win in app.windows.values_mut() {
            win.pump();
        }
        app.reap_dead();

        let (rows, cols) = terminal_size(terminal)?;
        let size = ratatui::layout::Size {
            width: cols,
            height: rows,
        };
        let content_height = size.height.saturating_sub(1).max(1);
        let full_area = Rect::new(0, 0, size.width, content_height);
        let layout = app.layout(full_area);
        for (id, rect) in &layout {
            if let Some(win) = app.windows.get_mut(id) {
                let inner_h = rect.height.saturating_sub(2).max(1);
                let inner_w = rect.width.saturating_sub(2).max(1);
                win.resize(inner_h, inner_w);
            }
        }

        let focused_id = app.focused_window_id();
        terminal.draw(|f| {
            for (position, (id, rect)) in layout.iter().enumerate() {
                if let Some(win) = app.windows.get(id) {
                    let widget = pty_widget::PtyWidget::new(
                        win.parser.screen(),
                        Some(*id) == focused_id,
                        position + 1,
                        accent,
                    );
                    f.render_widget(widget, *rect);
                }
            }

            let label_width = (VERSION_LABEL.len() as u16).min(size.width);
            let label_area = Rect::new(
                size.width.saturating_sub(label_width),
                size.height.saturating_sub(1),
                label_width,
                1,
            );
            f.render_widget(
                Paragraph::new(VERSION_LABEL).style(Style::default().fg(Color::DarkGray)),
                label_area,
            );

            let corner_text = if app.resize_mode {
                Some((resize_hint.as_str(), Color::Yellow))
            } else if !statusbar_text.is_empty() {
                Some((statusbar_text.as_str(), Color::Gray))
            } else {
                None
            };
            if let Some((text, color)) = corner_text {
                let max_width = size.width.saturating_sub(label_width + 1);
                let text: String = text.chars().take(max_width as usize).collect();
                let corner_area = Rect::new(0, size.height.saturating_sub(1), max_width, 1);
                f.render_widget(
                    Paragraph::new(text).style(Style::default().fg(color)),
                    corner_area,
                );
            }

            if let Some(fid) = focused_id
                && let Some((_, rect)) = layout.iter().find(|(id, _)| *id == fid)
                && let Some(win) = app.windows.get(&fid)
            {
                let screen = win.parser.screen();
                if !screen.hide_cursor() {
                    let (row, col) = screen.cursor_position();
                    let inner = pty_widget::inner_area(*rect);
                    if col < inner.width && row < inner.height {
                        f.set_cursor_position((inner.x + col, inner.y + row));
                    }
                }
            }
        })?;

        if event::poll(Duration::from_millis(33))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let was_resizing = app.resize_mode;
            handle_key(&mut app, &keybinds, key, full_area, &layout)?;
            if was_resizing && !app.resize_mode {
                // Resize mode can trigger many rapid pty resizes in a row;
                // force a full repaint on exit so any stale fragments left
                // by a child's own imperfect redraw (see the resize-nudge
                // comment in pty_window.rs) don't linger after leaving it.
                // `resize` (not `clear`) deliberately: for a fullscreen
                // viewport it just resets the internal diff buffer, with
                // no cursor-position query/round-trip to the terminal.
                terminal.resize(Rect::new(0, 0, size.width, size.height))?;
            }
        }

        if app.should_quit {
            break;
        }
    }

    // The statusbar process isn't a pty-backed pane, so it has no
    // controlling-terminal-hangup mechanism to clean it up on its own;
    // kill it explicitly so it doesn't linger after CLIWM exits.
    if let Some(mut child) = statusbar_child {
        let _ = child.kill();
        let _ = child.wait();
    }

    Ok(())
}

fn handle_key(
    app: &mut App,
    keybinds: &Keybinds,
    key: KeyEvent,
    content_area: Rect,
    layout: &[(usize, Rect)],
) -> Result<()> {
    if app.resize_mode {
        // Strictly modal: only the arrows resize and only `resize_exit`
        // leaves. Everything else -- including re-pressing the chord that
        // opened resize mode -- is ignored, so a stray keystroke can't
        // leak through to the terminal or silently exit.
        let chord = Chord::new(key.modifiers, key.code);
        if chord == keybinds.resize_exit {
            app.resize_mode = false;
            return Ok(());
        }
        match key.code {
            KeyCode::Up => app.resize_focused(Direction::Up, content_area),
            KeyCode::Down => app.resize_focused(Direction::Down, content_area),
            KeyCode::Left => app.resize_focused(Direction::Left, content_area),
            KeyCode::Right => app.resize_focused(Direction::Right, content_area),
            _ => {}
        }
        return Ok(());
    }

    if app.leader_pending {
        app.leader_pending = false;
        if key.modifiers == keybinds.leader.mods
            && let KeyCode::Char(c) = key.code
            && let Some(n) = c.to_digit(10)
            && (1..=9).contains(&n)
        {
            app.switch_workspace(n as usize - 1);
            return Ok(());
        }
        // Not a workspace digit held with the same modifier as the leader:
        // this keystroke wasn't part of the chord, so let it fall through
        // to normal handling instead of swallowing it.
    }

    let chord = Chord::new(key.modifiers, key.code);

    if chord == keybinds.leader {
        app.leader_pending = true;
        return Ok(());
    }

    if let Some(action) = keybinds.binds.get(&chord) {
        match action {
            Action::NewTerminal => app.new_terminal(content_area)?,
            Action::MoveWindow(dir) => app.move_focused(*dir, content_area),
            Action::KillWindow => app.kill_focused(),
            Action::Quit => app.should_quit = true,
            Action::SwitchWorkspace(n) => app.switch_workspace(n - 1),
            Action::ToggleResizeMode => app.resize_mode = !app.resize_mode,
        }
        return Ok(());
    }

    if key.modifiers == KeyModifiers::ALT
        && let KeyCode::Char(c) = key.code
        && let Some(n) = c.to_digit(10)
        && (1..=9).contains(&n)
        && let Some((id, _)) = layout.get(n as usize - 1)
    {
        app.focus_window(*id);
        return Ok(());
    }

    if let Some(id) = app.focused_window_id()
        && let Some(bytes) = key_event_to_bytes(key)
        && let Some(win) = app.windows.get_mut(&id)
    {
        win.write_input(&bytes);
    }
    Ok(())
}

/// Encodes a key event as the bytes a real terminal would send for it, so
/// that an Alt-modified key not bound to a CLIWM action (e.g. a shell's own
/// `Alt+F`/`Alt+B` word-jump bindings) still reaches the child with its Alt
/// prefix intact rather than silently losing it.
fn key_event_to_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char(c) = key.code
        && c.is_ascii_alphabetic()
    {
        let upper = c.to_ascii_uppercase() as u8;
        return Some(vec![upper & 0x1f]);
    }

    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            let bytes = c.encode_utf8(&mut buf).as_bytes().to_vec();
            Some(if alt { esc_prefixed(bytes) } else { bytes })
        }
        KeyCode::Enter => Some(if alt {
            vec![0x1b, b'\r']
        } else {
            b"\r".to_vec()
        }),
        KeyCode::Backspace => Some(if alt { vec![0x1b, 0x7f] } else { vec![0x7f] }),
        KeyCode::Tab => Some(if alt {
            vec![0x1b, b'\t']
        } else {
            b"\t".to_vec()
        }),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(arrow_bytes(b'A', alt)),
        KeyCode::Down => Some(arrow_bytes(b'B', alt)),
        KeyCode::Right => Some(arrow_bytes(b'C', alt)),
        KeyCode::Left => Some(arrow_bytes(b'D', alt)),
        _ => None,
    }
}

fn esc_prefixed(mut bytes: Vec<u8>) -> Vec<u8> {
    let mut out = vec![0x1b];
    out.append(&mut bytes);
    out
}

fn arrow_bytes(letter: u8, alt: bool) -> Vec<u8> {
    if alt {
        format!("\x1b[1;3{}", letter as char).into_bytes()
    } else {
        vec![0x1b, b'[', letter]
    }
}

/// Renders a chord as a human-readable string like "Alt+r" or "Ctrl+Up",
/// for showing keybind hints in the UI.
fn format_chord(chord: &Chord) -> String {
    let mut parts = Vec::new();
    if chord.mods.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl".to_string());
    }
    if chord.mods.contains(KeyModifiers::ALT) {
        parts.push("Alt".to_string());
    }
    if chord.mods.contains(KeyModifiers::SHIFT) {
        parts.push("Shift".to_string());
    }
    parts.push(match chord.code {
        KeyCode::Char(c) => c.to_uppercase().to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        _ => "?".to_string(),
    });
    parts.join("+")
}
