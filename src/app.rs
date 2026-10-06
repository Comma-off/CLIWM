use std::collections::HashMap;

use anyhow::Result;
use ratatui::layout::Rect;

use crate::keybinds::Direction;
use crate::layout::{BspNode, LayoutMode, MIN_PANE_WIDTH, grid_columns, grid_layout};
use crate::pty_window::PtyWindow;

const WORKSPACE_COUNT: usize = 9;
/// Matches the 1-9 digit range used to focus a window, so every window in a
/// workspace always has a reachable focus keybind.
const MAX_WINDOWS_PER_WORKSPACE: usize = 9;

#[derive(Default)]
pub struct Workspace {
    /// Window ids in creation order. Used for existence/cap bookkeeping,
    /// and directly as Grid mode's row-major reading order.
    pub windows: Vec<usize>,
    /// The focused window's id -- not an index, since BSP/Grid layouts
    /// don't have a single stable "position" a plain index could mean
    /// once creation order and visual order can diverge.
    pub focused: Option<usize>,
    /// The tiling tree for OneSide/Divide modes; stays `None` in Grid
    /// mode, which recomputes its arrangement from `windows` fresh every
    /// call instead.
    bsp: Option<BspNode>,
}

impl Workspace {
    fn layout(&self, mode: LayoutMode, area: Rect) -> Vec<(usize, Rect)> {
        match mode {
            LayoutMode::Grid => grid_layout(&self.windows, area),
            LayoutMode::OneSide | LayoutMode::Divide => self
                .bsp
                .as_ref()
                .map(|tree| tree.layout(area))
                .unwrap_or_default(),
        }
    }
}

pub struct App {
    pub windows: HashMap<usize, PtyWindow>,
    pub workspaces: Vec<Workspace>,
    pub current_workspace: usize,
    pub leader_pending: bool,
    pub resize_mode: bool,
    pub layout_mode: LayoutMode,
    next_id: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new(layout_mode: LayoutMode) -> Self {
        let workspaces = (0..WORKSPACE_COUNT).map(|_| Workspace::default()).collect();
        Self {
            windows: HashMap::new(),
            workspaces,
            current_workspace: 0,
            leader_pending: false,
            resize_mode: false,
            layout_mode,
            next_id: 0,
            should_quit: false,
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspaces[self.current_workspace]
    }

    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.current_workspace]
    }

    pub fn focused_window_id(&self) -> Option<usize> {
        self.workspace().focused
    }

    /// Computes the current workspace's layout for `area`, in
    /// left-to-right/top-to-bottom reading order (the same order used for
    /// on-screen numbering and digit-focus).
    pub fn layout(&self, area: Rect) -> Vec<(usize, Rect)> {
        self.workspace().layout(self.layout_mode, area)
    }

    pub fn new_terminal(&mut self, area: Rect) -> Result<()> {
        if self.workspace().windows.len() >= MAX_WINDOWS_PER_WORKSPACE {
            return Ok(());
        }
        // Spawn (fallible) before touching any bookkeeping, so a failure
        // here can't leave the tree referencing an id that was never
        // actually created.
        let window = PtyWindow::spawn(area.height.max(1), area.width.max(1))?;
        let id = self.next_id;
        self.next_id += 1;
        self.windows.insert(id, window);

        let mode = self.layout_mode;
        let ws = self.workspace_mut();
        if ws.windows.is_empty() {
            ws.bsp = Some(BspNode::leaf(id));
        } else if mode != LayoutMode::Grid {
            let target = ws.focused.unwrap_or(ws.windows[0]);
            let horizontal = match mode {
                LayoutMode::OneSide => true,
                LayoutMode::Divide => {
                    let target_rect = ws
                        .bsp
                        .as_ref()
                        .and_then(|tree| tree.rect_of(target, area))
                        .unwrap_or(area);
                    prefer_horizontal_split(target_rect)
                }
                LayoutMode::Grid => unreachable!(),
            };
            if let Some(tree) = ws.bsp.as_mut() {
                tree.insert(target, id, horizontal);
            }
        }
        // Grid mode needs no tree update: it recomputes from `windows`.

        ws.windows.push(id);
        ws.focused = Some(id);
        Ok(())
    }

    pub fn kill_focused(&mut self) {
        let Some(id) = self.focused_window_id() else {
            return;
        };
        if let Some(win) = self.windows.get_mut(&id) {
            win.kill();
        }
        self.remove_window(id);
    }

    /// Drops dead child processes from the current workspace, adjusting focus.
    pub fn reap_dead(&mut self) {
        let mut dead = Vec::new();
        for (id, win) in self.windows.iter_mut() {
            if !win.is_alive() {
                dead.push(*id);
            }
        }
        for id in dead {
            self.remove_window(id);
        }
    }

    fn remove_window(&mut self, id: usize) {
        self.windows.remove(&id);
        for ws in &mut self.workspaces {
            if let Some(pos) = ws.windows.iter().position(|w| *w == id) {
                ws.windows.remove(pos);
                ws.bsp = ws.bsp.take().and_then(|tree| tree.remove(id));
                if ws.focused == Some(id) {
                    ws.focused = if ws.windows.is_empty() {
                        None
                    } else {
                        Some(ws.windows[pos.min(ws.windows.len() - 1)])
                    };
                }
            }
        }
    }

    /// Focuses `id` if it exists in the current workspace.
    pub fn focus_window(&mut self, id: usize) {
        let ws = self.workspace_mut();
        if ws.windows.contains(&id) {
            ws.focused = Some(id);
        }
    }

    pub fn switch_workspace(&mut self, index: usize) {
        if index < self.workspaces.len() {
            self.current_workspace = index;
        }
    }

    /// Swaps the focused window with a neighbor in the given direction.
    /// Focus stays on the moved window. In Grid mode this swaps grid
    /// cells; in OneSide/Divide it swaps within the tiling tree (see
    /// [`BspNode::move_focused`]).
    pub fn move_focused(&mut self, dir: Direction, area: Rect) {
        let mode = self.layout_mode;
        let ws = self.workspace_mut();
        let Some(focused) = ws.focused else {
            return;
        };

        match mode {
            LayoutMode::Grid => {
                let Some(idx) = ws.windows.iter().position(|w| *w == focused) else {
                    return;
                };
                let cols = grid_columns(ws.windows.len(), area.width);
                let len = ws.windows.len();
                let target = match dir {
                    Direction::Left if idx % cols != 0 => Some(idx - 1),
                    Direction::Right if (idx + 1) % cols != 0 && idx + 1 < len => Some(idx + 1),
                    Direction::Up if idx >= cols => Some(idx - cols),
                    Direction::Down if idx + cols < len => Some(idx + cols),
                    _ => None,
                };
                if let Some(target) = target {
                    ws.windows.swap(idx, target);
                }
            }
            LayoutMode::OneSide | LayoutMode::Divide => {
                if let Some(tree) = ws.bsp.as_mut() {
                    tree.move_focused(focused, dir);
                }
            }
        }
    }

    /// Grows the focused window by one column/row in `dir`, shrinking a
    /// neighbor to match. No-op in Grid mode, where every cell is uniform.
    /// See [`BspNode::resize`] for the OneSide/Divide semantics.
    pub fn resize_focused(&mut self, dir: Direction, area: Rect) {
        if self.layout_mode == LayoutMode::Grid {
            return;
        }
        let ws = self.workspace_mut();
        let Some(focused) = ws.focused else {
            return;
        };
        if let Some(tree) = ws.bsp.as_mut() {
            tree.resize(focused, dir, area);
        }
    }
}

/// Decides Divide mode's split direction for a newly split pane: rows if
/// the pane is too narrow to give both halves at least [`MIN_PANE_WIDTH`],
/// otherwise whichever direction leaves the two halves closer to square
/// (terminal cells are roughly twice as tall as wide, so width is halved
/// before comparing).
fn prefer_horizontal_split(rect: Rect) -> bool {
    if rect.width < MIN_PANE_WIDTH * 2 {
        return true;
    }
    rect.width / 2 <= rect.height
}
