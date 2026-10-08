//! Shared terminal dialog chrome. Layout is content-sized and terminal-theme aware.
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph},
};

#[derive(Clone, Copy)]
pub enum Tone {
    Normal,
    Danger,
}

/// Render a centered panel and return the padded body above its action footer.
pub fn panel(
    frame: &mut Frame,
    title: &str,
    width: u16,
    content_height: u16,
    footer: &str,
    tone: Tone,
) -> Rect {
    let screen = frame.area();
    let width = width.min(
        screen
            .width
            .saturating_sub(if screen.width > 8 { 4 } else { 0 }),
    );
    let height =
        content_height
            .saturating_add(4)
            .min(
                screen
                    .height
                    .saturating_sub(if screen.height > 8 { 2 } else { 0 }),
            );
    let area = Rect::new(
        screen.x + screen.width.saturating_sub(width) / 2,
        screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    );
    // Dim the underlying listing, preserving the user's terminal palette.
    for y in screen.y..screen.bottom() {
        for x in screen.x..screen.right() {
            frame.buffer_mut()[(x, y)].set_style(Style::default().add_modifier(Modifier::DIM));
        }
    }
    frame.render_widget(Clear, area);
    let accent = match tone {
        Tone::Normal => Color::Cyan,
        Tone::Danger => Color::Red,
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(
            Style::default()
                .fg(Color::Reset)
                .bg(Color::Reset)
                .remove_modifier(Modifier::DIM),
        )
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                title.to_owned(),
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inset = u16::from(inner.width > 4);
    let body = Rect::new(
        inner.x + inset,
        inner.y,
        inner.width.saturating_sub(inset * 2),
        inner.height.saturating_sub(2),
    );
    if inner.height >= 2 {
        frame.render_widget(
            Paragraph::new("─".repeat(usize::from(inner.width)))
                .style(Style::default().fg(Color::DarkGray)),
            Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
        );
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(accent)),
            Rect::new(body.x, inner.bottom() - 1, body.width, 1),
        );
    }
    body
}

/// Render an active one-line input, scrolling by terminal cells rather than bytes.
pub fn input(frame: &mut Frame, area: Rect, value: &str) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let line = Line::from(super::view::display_os(std::ffi::OsStr::new(value)));
    let width = line.width();
    let visible = usize::from(area.width.saturating_sub(1));
    frame.render_widget(
        Paragraph::new(line)
            .style(
                Style::default()
                    .fg(Color::Reset)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::UNDERLINED),
            )
            .scroll((
                0,
                width.saturating_sub(visible).min(u16::MAX as usize) as u16,
            )),
        Rect::new(area.x, area.y, area.width, 1),
    );
    frame.set_cursor_position(Position::new(area.x + width.min(visible) as u16, area.y));
}

#[cfg(test)]
pub fn capture(name: &str, buffer: &ratatui::buffer::Buffer) {
    let Ok(directory) = std::env::var("EXCAVATOR_MODAL_SNAPSHOTS_DIR") else {
        return;
    };
    let cells: Vec<_> = buffer.content.iter().map(|cell| serde_json::json!({"text":cell.symbol(), "fg":format!("{:?}",cell.fg), "bg":format!("{:?}",cell.bg), "modifier":format!("{:?}",cell.modifier)})).collect();
    let value =
        serde_json::json!({"width":buffer.area.width,"height":buffer.area.height,"cells":cells});
    std::fs::create_dir_all(&directory).expect("snapshot directory");
    std::fs::write(
        std::path::Path::new(&directory).join(format!("{name}.json")),
        serde_json::to_vec(&value).unwrap(),
    )
    .expect("snapshot file");
}
