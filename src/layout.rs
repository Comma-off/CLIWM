use ratatui::layout::Rect;

use crate::keybinds::Direction;

const MIN_RATIO: f32 = 0.05;
/// Below this width, a column is narrower than many full-screen terminal
/// apps require (e.g. btop and several other ncurses tools refuse to
/// render below 80 columns). Grid caps its column count, and Divide falls
/// back to a row split, to avoid creating columns under this width.
pub const MIN_PANE_WIDTH: u16 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// dwm-style: each new window splits the currently focused pane,
    /// always into rows (top/bottom).
    OneSide,
    /// All windows recomputed fresh into a roughly-square grid every time,
    /// independent of creation order or focus history.
    Grid,
    /// Like `OneSide`, but each split's direction (rows vs columns) is
    /// chosen from the aspect ratio of the pane being split, producing a
    /// grid-like nested tiling that depends on creation order.
    Divide,
}

/// A binary space partition tree: each interior node splits its area into
/// two panes (rows or columns) at a given ratio; each leaf is a window id.
#[derive(Debug, Clone)]
pub enum BspNode {
    Leaf(usize),
    Split {
        /// true: split into top/bottom (rows). false: left/right (columns).
        horizontal: bool,
        /// `first`'s share of the split, 0.0-1.0.
        ratio: f32,
        first: Box<BspNode>,
        second: Box<BspNode>,
    },
}

impl BspNode {
    pub fn leaf(id: usize) -> Self {
        BspNode::Leaf(id)
    }

    pub fn contains(&self, id: usize) -> bool {
        match self {
            BspNode::Leaf(leaf_id) => *leaf_id == id,
            BspNode::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// Replaces the leaf `target` with a split containing the old leaf and
    /// a new leaf for `new_id`. Returns true if `target` was found.
    pub fn insert(&mut self, target: usize, new_id: usize, horizontal: bool) -> bool {
        match self {
            BspNode::Leaf(id) => {
                if *id != target {
                    return false;
                }
                *self = BspNode::Split {
                    horizontal,
                    ratio: 0.5,
                    first: Box::new(BspNode::Leaf(*id)),
                    second: Box::new(BspNode::Leaf(new_id)),
                };
                true
            }
            BspNode::Split { first, second, .. } => {
                first.insert(target, new_id, horizontal)
                    || second.insert(target, new_id, horizontal)
            }
        }
    }

    /// Removes `id` from the tree, promoting its sibling in its place.
    /// Returns `None` if `id` was the entire tree.
    pub fn remove(self, id: usize) -> Option<BspNode> {
        match self {
            BspNode::Leaf(leaf_id) => (leaf_id != id).then_some(BspNode::Leaf(leaf_id)),
            BspNode::Split {
                horizontal,
                ratio,
                first,
                second,
            } => match (*first, *second) {
                (BspNode::Leaf(fid), other) if fid == id => Some(other),
                (other, BspNode::Leaf(sid)) if sid == id => Some(other),
                (first, second) => {
                    let new_first = first.remove(id);
                    let new_second = second.remove(id);
                    match (new_first, new_second) {
                        (Some(f), Some(s)) => Some(BspNode::Split {
                            horizontal,
                            ratio,
                            first: Box::new(f),
                            second: Box::new(s),
                        }),
                        (Some(f), None) => Some(f),
                        (None, Some(s)) => Some(s),
                        (None, None) => None,
                    }
                }
            },
        }
    }

    /// Computes each leaf's rect, in left-to-right/top-to-bottom reading
    /// order (matching how windows are numbered on screen).
    pub fn layout(&self, area: Rect) -> Vec<(usize, Rect)> {
        match self {
            BspNode::Leaf(id) => vec![(*id, area)],
            BspNode::Split {
                horizontal,
                ratio,
                first,
                second,
            } => {
                let (first_area, second_area) = split_rect(area, *horizontal, *ratio);
                let mut result = first.layout(first_area);
                result.extend(second.layout(second_area));
                result
            }
        }
    }

    /// Finds `target`'s rect within `area` without computing the whole
    /// tree's layout.
    pub fn rect_of(&self, target: usize, area: Rect) -> Option<Rect> {
        match self {
            BspNode::Leaf(id) => (*id == target).then_some(area),
            BspNode::Split {
                horizontal,
                ratio,
                first,
                second,
            } => {
                let (first_area, second_area) = split_rect(area, *horizontal, *ratio);
                first
                    .rect_of(target, first_area)
                    .or_else(|| second.rect_of(target, second_area))
            }
        }
    }

    /// Grows `target` by one column/row in `dir`, shrinking a neighbor to
    /// match, at the nearest ancestor split whose orientation matches the
    /// direction. Returns true if a resize was applied (false if `target`
    /// has no such ancestor, e.g. it's the whole tree, or `dir` never
    /// finds a matching split up its path).
    pub fn resize(&mut self, target: usize, dir: Direction, area: Rect) -> bool {
        let BspNode::Split {
            horizontal,
            ratio,
            first,
            second,
        } = self
        else {
            return false;
        };
        let (first_area, second_area) = split_rect(area, *horizontal, *ratio);
        let target_in_first = first.contains(target);
        if !target_in_first && !second.contains(target) {
            return false;
        }

        let handled_deeper = if target_in_first {
            first.resize(target, dir, first_area)
        } else {
            second.resize(target, dir, second_area)
        };
        if handled_deeper {
            return true;
        }

        let orientation_matches = match dir {
            Direction::Up | Direction::Down => *horizontal,
            Direction::Left | Direction::Right => !*horizontal,
        };
        if !orientation_matches {
            return false;
        }

        let step = if *horizontal {
            1.0 / area.height.max(1) as f32
        } else {
            1.0 / area.width.max(1) as f32
        };
        // `first` is top/left, `second` is bottom/right. Pressing the
        // direction that points from `target` toward its sibling grows
        // `target`; pressing away from the sibling shrinks it (there's
        // nothing beyond `target`'s own edge at this level to grow into).
        let target_grows = matches!(
            (target_in_first, dir),
            (true, Direction::Down)
                | (true, Direction::Right)
                | (false, Direction::Up)
                | (false, Direction::Left)
        );
        // `ratio` is always `first`'s share, so growing `second` means
        // *decreasing* it -- only growing `first` itself increases it.
        let ratio_increases = target_grows == target_in_first;
        *ratio = if ratio_increases {
            (*ratio + step).min(1.0 - MIN_RATIO)
        } else {
            (*ratio - step).max(MIN_RATIO)
        };
        true
    }

    /// Swaps `target` with its sibling at the nearest ancestor split whose
    /// orientation and side match `dir`. Returns true if a swap happened.
    pub fn move_focused(&mut self, target: usize, dir: Direction) -> bool {
        let BspNode::Split {
            horizontal,
            first,
            second,
            ..
        } = self
        else {
            return false;
        };
        let target_in_first = first.contains(target);
        if !target_in_first && !second.contains(target) {
            return false;
        }

        let handled_deeper = if target_in_first {
            first.move_focused(target, dir)
        } else {
            second.move_focused(target, dir)
        };
        if handled_deeper {
            return true;
        }

        let orientation_matches = match dir {
            Direction::Up | Direction::Down => *horizontal,
            Direction::Left | Direction::Right => !*horizontal,
        };
        if !orientation_matches {
            return false;
        }
        let should_swap = matches!(
            (target_in_first, dir),
            (true, Direction::Down)
                | (true, Direction::Right)
                | (false, Direction::Up)
                | (false, Direction::Left)
        );
        if should_swap {
            std::mem::swap(first, second);
        }
        should_swap
    }
}

fn split_rect(area: Rect, horizontal: bool, ratio: f32) -> (Rect, Rect) {
    if horizontal {
        let first_h = ((area.height as f32) * ratio)
            .round()
            .clamp(1.0, area.height.saturating_sub(1).max(1) as f32) as u16;
        let first = Rect {
            height: first_h,
            ..area
        };
        let second = Rect {
            y: area.y + first_h,
            height: area.height.saturating_sub(first_h),
            ..area
        };
        (first, second)
    } else {
        let first_w = ((area.width as f32) * ratio)
            .round()
            .clamp(1.0, area.width.saturating_sub(1).max(1) as f32) as u16;
        let first = Rect {
            width: first_w,
            ..area
        };
        let second = Rect {
            x: area.x + first_w,
            width: area.width.saturating_sub(first_w),
            ..area
        };
        (first, second)
    }
}

/// Arranges `ids` into a roughly-square grid (row-major), recomputed fresh
/// from the current window list every call -- no persistent tree, no
/// dependency on creation order beyond the reading order it produces.
pub fn grid_layout(ids: &[usize], area: Rect) -> Vec<(usize, Rect)> {
    let n = ids.len();
    if n == 0 {
        return Vec::new();
    }
    let cols = grid_columns(n, area.width);
    let rows = n.div_ceil(cols);

    let row_height = area.height / rows as u16;
    let mut result = Vec::with_capacity(n);
    for r in 0..rows {
        let y = area.y + row_height * r as u16;
        let h = if r == rows - 1 {
            area.height - row_height * r as u16
        } else {
            row_height
        };
        let row_start = r * cols;
        let items_in_row = cols.min(n - row_start);
        let col_width = area.width / items_in_row as u16;
        for c in 0..items_in_row {
            let x = area.x + col_width * c as u16;
            let w = if c == items_in_row - 1 {
                area.width - col_width * c as u16
            } else {
                col_width
            };
            result.push((
                ids[row_start + c],
                Rect {
                    x,
                    y,
                    width: w,
                    height: h,
                },
            ));
        }
    }
    result
}

/// Number of columns in an `n`-window grid: the smallest square that fits,
/// capped so columns don't drop below [`MIN_PANE_WIDTH`] when the area is
/// too narrow for a fully square arrangement.
pub fn grid_columns(n: usize, area_width: u16) -> usize {
    let square = (n as f32).sqrt().ceil() as usize;
    let max_by_width = (area_width / MIN_PANE_WIDTH).max(1) as usize;
    square.min(max_by_width).max(1)
}
