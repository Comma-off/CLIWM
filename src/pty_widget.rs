use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color as RColor, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

pub struct PtyWidget<'a> {
    screen: &'a vt100::Screen,
    focused: bool,
    number: usize,
    accent: RColor,
}

impl<'a> PtyWidget<'a> {
    pub fn new(screen: &'a vt100::Screen, focused: bool, number: usize, accent: RColor) -> Self {
        Self {
            screen,
            focused,
            number,
            accent,
        }
    }
}

/// The content area inside a pane's border, shared with cursor placement in
/// `main.rs` so the two never disagree about where cell (0, 0) lands.
pub fn inner_area(area: Rect) -> Rect {
    Block::default().borders(Borders::ALL).inner(area)
}

fn conv_color(c: vt100::Color) -> Option<RColor> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(RColor::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(RColor::Rgb(r, g, b)),
    }
}

impl<'a> Widget for PtyWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            Style::default().fg(self.accent)
        } else {
            Style::default().fg(RColor::DarkGray)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(format!(" {} ", self.number));
        let inner = inner_area(area);
        block.render(area, buf);

        let (rows, cols) = self.screen.size();
        for row in 0..rows.min(inner.height) {
            for col in 0..cols.min(inner.width) {
                let Some(cell) = self.screen.cell(row, col) else {
                    continue;
                };
                let x = inner.x + col;
                let y = inner.y + row;

                let symbol = if cell.has_contents() {
                    cell.contents()
                } else {
                    " "
                };

                let mut style = Style::default();
                if let Some(fg) = conv_color(cell.fgcolor()) {
                    style = style.fg(fg);
                }
                if let Some(bg) = conv_color(cell.bgcolor()) {
                    style = style.bg(bg);
                }
                if cell.bold() {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if cell.italic() {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if cell.underline() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if cell.inverse() {
                    style = style.add_modifier(Modifier::REVERSED);
                }

                buf.set_string(x, y, symbol, style);
            }
        }
    }
}
