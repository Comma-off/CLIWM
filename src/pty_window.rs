use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Tracks "synchronized output" mode (`CSI ? 2026 h`/`l`), a DEC private
/// mode `vt100` doesn't know about and safely ignores, but that some modern
/// full-screen programs (e.g. btop) use to mark a batch of escape codes as
/// one atomic screen update. Without tracking it ourselves, our render loop
/// -- which runs independently of the child's update rhythm -- can capture
/// and display a mid-update, visually torn frame.
#[derive(Default)]
pub(crate) struct SyncTracker {
    in_sync: bool,
}

impl vt100::Callbacks for SyncTracker {
    fn unhandled_csi(
        &mut self,
        _screen: &mut vt100::Screen,
        intermediate: Option<u8>,
        _intermediate2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        if intermediate == Some(b'?') && params == [[2026]] {
            self.in_sync = c == 'h';
        }
    }
}

pub struct PtyWindow {
    pub parser: vt100::Parser<SyncTracker>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    output_rx: Receiver<Vec<u8>>,
    pub rows: u16,
    pub cols: u16,
    ever_resized: bool,
}

impl PtyWindow {
    pub fn spawn(rows: u16, cols: u16) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let cmd = CommandBuilder::new(shell);
        let child = pair
            .slave
            .spawn_command(cmd)
            .context("spawning shell in pty")?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            parser: vt100::Parser::new_with_callbacks(rows, cols, 2000, SyncTracker::default()),
            writer,
            master: pair.master,
            child,
            output_rx: rx,
            rows,
            cols,
            ever_resized: false,
        })
    }

    /// Drains any pty output received since the last call and feeds it into
    /// the vt100 parser. Returns true if anything changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok(bytes) = self.output_rx.try_recv() {
            self.parser.process(&bytes);
            changed = true;
        }

        // If a synchronized-output block (see `SyncTracker`) is still open,
        // give the writer a brief chance to finish it rather than rendering
        // a torn mid-update frame. The whole burst is normally written in
        // one shot, so this rarely spins more than once or twice; bounded
        // so a program that opens sync mode and never closes it can't stall
        // the UI forever.
        let mut attempts = 0;
        while self.parser.callbacks().in_sync && attempts < 50 {
            match self.output_rx.recv_timeout(Duration::from_millis(2)) {
                Ok(bytes) => self.parser.process(&bytes),
                Err(_) => break,
            }
            attempts += 1;
        }

        changed
    }

    pub fn write_input(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
    }

    /// Nudges the shell to clear and redraw its current line. See the
    /// comment in `resize` for why this is needed.
    pub fn nudge_redraw(&mut self) {
        let _ = self.writer.write_all(&[0x0c]);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if rows == self.rows && cols == self.cols {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.parser.screen_mut().set_size(rows, cols);

        // Many shell prompts (p10k, starship, ...) redraw a width-dependent
        // line on SIGWINCH without clearing to end-of-line first, which can
        // leave stale fragments behind after a resize. Ctrl+L is the
        // conventional "clear & redraw" byte honored by readline/zle and
        // most full-screen programs, so nudge an immediate clean repaint
        // instead of waiting for the next keystroke. Skip this on the very
        // first resize: the shell may not have finished setting up its own
        // input mode yet, and the byte can leak through as literal "^L".
        if self.ever_resized {
            self.nudge_redraw();
        }
        self.ever_resized = true;
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}
