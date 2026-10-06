# CLIWM

A tiling window manager that runs inside a terminal emulator. Not meant for
daily driving — a sandbox for running a handful of CLI programs as tiled
panes without leaving the terminal.

Each pane is a real PTY running your `$SHELL`, rendered via a vt100 terminal
emulator ([`vt100`](https://crates.io/crates/vt100)) into a
[`ratatui`](https://crates.io/crates/ratatui) UI. A pane's PTY is tracked
for "synchronized output" mode (`CSI ?2026h`/`l`), a batching hint some
modern full-screen apps (e.g. `btop`) send around a screen update; CLIWM
waits briefly for the matching end marker before rendering that pane again,
so redraws aren't shown mid-update. The real terminal cursor is shown at
the focused pane's cursor position (and hidden when that program hides its
own cursor, e.g. full-screen TUI apps).

## Running

```sh
cargo run --release
```

## Default keybinds

| Key                                     | Action                                   |
| ---------------------------------------- | ------------------------------------------ |
| `Alt+t`                                  | New terminal                             |
| `Alt+Up`/`Down`/`Left`/`Right`           | Move the focused window in the layout    |
| `Alt+1`-`9`                              | Focus window N in the current workspace  |
| `Alt+w`, then (still holding Alt) `1`-`9` | Switch to workspace N                    |
| `Alt+r`, then arrows, then `Esc`          | Resize mode (see below)                  |
| `Alt+x`                                  | Kill the focused window                  |
| `Alt+q`                                  | Quit CLIWM                                |

Every default bind uses `Alt` — Ctrl is deliberately left free for shell
and readline shortcuts (`Ctrl+W`, `Ctrl+C`, etc.), since those are far more
commonly relied on than Alt-based ones. Any key not bound to a WM action is
forwarded to the focused terminal as input with its modifiers intact —
including plain arrow keys, plain `w`, and plain digits, which reach the
terminal as normal (e.g. for shell history, editor movement, or just
typing numbers) since every CLIWM bind requires a modifier held down.
Window-focus and workspace-switch digits specifically use `Alt` rather
than `Ctrl` because most terminals can't distinguish `Ctrl+1`/`2`/`3`/`8`/`9`
from other keys (they either collide with Esc/Backspace or arrive with no
modifier bit at all) — `Alt+<digit>` is the classic, universally-supported
encoding.

**Resize mode**: `Alt+r` toggles it on, shown as "Resize mode | Esc to
exit" in the bottom-left corner (using whatever chord is actually
configured as `resize_exit`, default `Esc`). It's strictly modal: while
active, only the arrow keys and `resize_exit` do anything — every other
key, including re-pressing `Alt+r`, is ignored, so a stray keystroke can't
leak through to the terminal or silently exit. Each arrow press moves a
one-column/row-wide boundary by one step, growing the window on one side
and shrinking the one on the other; see [Layouts](#layouts) for what
`Up`/`Down`/`Left`/`Right` mean in each layout. Exiting forces a full
repaint, clearing any stale fragments a rapid string of resizes can leave
behind from a pane's own imperfect redraw (see the `Ctrl+L` note in
[Known limitations](#known-limitations)).

Every keybind — including the workspace-switch leader, the exact workspace
it jumps to, and resize mode's exit key — is remappable. A workspace holds
at most 9 windows, matching the 1-9 digit range used to focus them. See
[Configuration](#configuration).

## Layouts

Set via `layout = one-side|grid|divide` in the config (default `one-side`).
A small `CLIWM v0.1` label is shown in the bottom-right corner regardless
of layout.

- **`one-side`** — dwm-style: each new window splits whichever pane is
  currently focused, always into rows (top/bottom). Opening several
  terminals without changing focus builds a top-to-bottom stack; focusing
  a specific pane first and opening a terminal splits just that pane.
  Resize mode's `Up`/`Down` grow/shrink a window against its row neighbor;
  `Left`/`Right` are a no-op once there's more than one window, since
  everything lives in a single column.
- **`grid`** — all windows recomputed fresh into a roughly-square grid
  every time, independent of focus or creation order (capped narrower than
  a full square if needed to keep columns at least 80 wide). Resize mode's
  arrows move focus between grid cells but don't resize — the grid is
  always uniform.
- **`divide`** — like `one-side`, but each split's direction (rows or
  columns) is chosen from the aspect ratio of the pane being split (falling
  back to rows if a column split would go under 80 wide), so the result
  tends to look grid-like without being a flat grid. All four resize-mode
  arrows work, adjusting whichever split they're nearest to.

For `one-side`/`divide`, `Up` moves the boundary *above* the focused window
up (growing it, shrinking its neighbor) — but if there's no neighbor above
it at that split, `Up` instead moves the boundary *below* it up (shrinking
the focused window, growing its neighbor there). `Down`/`Left`/`Right` are
the mirror image, in their respective directions. Moving a window (not
resizing) with `Alt`+arrow keys swaps it with a neighbor the same way,
except in `grid` mode, where it swaps with the adjacent grid cell.

## Configuration

Keybinds and the focused-pane accent color can be set in
`~/.config/cliwm.conf`, using a Hyprland-style format. That file is created
automatically (from [`cliwm.conf.example`](cliwm.conf.example)) the first
time you run CLIWM if it doesn't already exist.

```
accent = #61afef
```

sets the focused pane's border color (default: cyan).

The workspace-switch leader chord (`leader = alt, w` by default) is
remappable to any modifier + key, and the digit half always matches
whatever modifier the leader uses. If you'd rather jump to one specific
workspace directly instead of the two-key leader sequence, bind it with
`switch_workspace`:

```
bind = alt, f1, switch_workspace, 1
bind = alt, f2, switch_workspace, 2
```

`resize_exit` (default `esc`) sets resize mode's exit chord the same way.

See [`cliwm.conf.example`](cliwm.conf.example) for the full syntax,
including all bindable actions.

### Status bar

CLIWM doesn't ship a status bar, but it has a hook for building one — think
"a waybar, but for CLIWM". `statusbar` names a program CLIWM launches once,
independently of any workspace or pane (it isn't a terminal you can focus
or kill, and it's killed when CLIWM exits), and shows whatever line it
last printed in the bottom-left corner (the version label keeps the
bottom-right), updating whenever a new one arrives.

```
statusbar = ~/.config/cliwm-statusbar.sh
```

The program owns its own update cadence — an i3status/waybar-style script
that loops forever printing a fresh line every second (or only when
something actually changes) works well; CLIWM never re-invokes or polls
it, just reads its stdout continuously. There's no default script — this
is intentionally left for people to build.

## Known limitations

- Full-screen terminal apps with a minimum size (e.g. `btop` needs 80x24)
  can still show their own "terminal too small" message if the *overall*
  CLIWM window isn't big enough to give every open pane at least that much
  room — this is inherent to any tiling layout (the same happens in i3,
  tmux, etc.) and can't be avoided by rearranging panes alone.
- In `one-side`/`divide`, repeatedly opening new terminals without changing
  focus keeps halving the same (most recently focused) pane, so later
  windows can end up too short to show a legible border/title if you open
  many in a short terminal. `grid` distributes space more evenly for that
  case.
- No mouse support.
- Double-width (CJK) characters are not handled specially.
- Shell prompts that redraw width-dependent content on resize (Powerlevel10k,
  Starship, etc.) can leave brief visual fragments right after a pane is
  resized (e.g. from closing a neighbor). CLIWM sends a `Ctrl+L` redraw nudge
  to just the resized pane to help, but this depends on how your specific
  prompt theme handles `SIGWINCH` — it isn't guaranteed to fully clear the
  fragments, which otherwise clear on the next keystroke.
