//! Rendering — maps shared state + mode to themed widgets.

use super::app::App;
use crate::core::message::Role;
use crate::mode::Mode;
use crate::theme::Palette;
use crate::APP_NAME;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &App) {
    let palette = Palette::for_mode(app.mode);
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(f, chunks[0], app, palette);
    draw_messages(f, chunks[1], app, palette);
    draw_input(f, chunks[2], app, palette);
    draw_footer(f, chunks[3], app, palette);

    // Overlays layered on top of the base layout.
    if app.perm_pending() {
        if let Some(req) = app.permission_request() {
            draw_permission_dialog(f, area, &req, palette);
        }
    } else if let Some((sel, items)) = app.palette() {
        draw_palette(f, area, sel, items, palette);
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let mode_badge = format!("[ {} ]", app.mode.label());
    let left = Line::from(vec![
        Span::styled(APP_NAME, Style::default().fg(p.accent_bright).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(mode_badge, mode_badge_style(app.mode, p)),
    ]);
    let right = Line::from(vec![
        Span::styled("Powered By Vexify.", Style::default().fg(p.dim)),
    ]);
    let line = merge_lines(left, right, area.width.saturating_sub(2));
    f.render_widget(
        Paragraph::new(line).block(header_block(p)),
        area,
    );
}

fn draw_messages(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let conv = app.shared.lock_conv();
    let mut lines: Vec<Line> = Vec::new();

    for m in conv.turns() {
        match m.role {
            Role::User => {
                lines.push(Line::styled(" you", Style::default().fg(p.user).add_modifier(Modifier::BOLD)));
                if let Some(c) = &m.content {
                    for l in c.lines() {
                        lines.push(Line::from(vec![Span::styled(format!("  {l}"), Style::default().fg(p.user))]));
                    }
                }
            }
            Role::Assistant => {
                lines.push(Line::styled(" ◈ OpenSauce", Style::default().fg(p.accent_bright)));
                if let Some(c) = &m.content {
                    for l in c.lines() {
                        lines.push(Line::from(vec![Span::styled(l.to_string(), Style::default().fg(p.assistant))]));
                    }
                }
                // streaming
                if app.shared.is_streaming() && app.shared.pending().is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("  ▸ thinking", Style::default().fg(p.dim)),
                    ]));
                }
            }
            Role::Tool => {
                if let Some(tr) = &m.tool_result {
                    lines.push(Line::styled(
                        format!(" ⇣ [{}] {}", if tr.success { "ok" } else { "err" }, tr.name),
                        Style::default().fg(if tr.success { p.accent_bright } else { p.error }),
                    ));
                    let text = tr.content.clone();
                    for l in text.lines().take(6) {
                        lines.push(Line::styled(format!("  {l}"), Style::default().fg(p.tool)));
                    }
                }
            }
            Role::System => {
                if let Some(c) = &m.content {
                    for l in c.lines() {
                        lines.push(Line::styled(format!("▸ {l}"), Style::default().fg(p.dim)));
                    }
                }
            }
        }
        lines.push(Line::raw(""));
    }

    // Pending streaming bubble.
    let pending = app.shared.pending();
    if app.shared.is_streaming() && !pending.is_empty() {
        lines.push(Line::styled(" ◈ OpenSauce", Style::default().fg(p.accent_bright)));
        for l in pending.lines() {
            lines.push(Line::styled(l.to_string(), Style::default().fg(p.assistant)));
        }
        lines.push(Line::raw(""));
    }

    drop(conv);

    let msg_area = inner(area);
    let total = lines.len().saturating_sub(1);
    let scroll = total.saturating_sub(msg_area.height as usize);

    let text = Text::from(lines);
    let para = Paragraph::new(text)
        .block(Block::default().padding(Padding::horizontal(1)))
        .wrap(Wrap { trim: true })
        .scroll((scroll as u16, 0));
    f.render_widget(para, msg_area);
}

fn draw_input(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let prefix = Span::styled(
        format!("> "),
        Style::default().fg(p.accent_bright).add_modifier(Modifier::BOLD),
    );
    let value = app.input.as_str();
    let prompt = Line::from(vec![
        prefix,
        Span::styled(value, Style::default().fg(p.assistant)),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(vec![
            Span::styled(" prompt ", Style::default().fg(p.accent_bright)),
        ]));
    f.render_widget(Paragraph::new(prompt).block(block), area);

    // Place the cursor just after the typed text.
    let inner = inner(area);
    let y = inner.y;
    let x = inner.x + 2 + app.input.chars().count() as u16;
    f.set_cursor_position(ratatui::layout::Position::new(
        x.min(inner.right().saturating_sub(1)),
        y,
    ));
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let status = app.shared.status_line();
    let mode = app.mode.label();
    let auto = app.is_auto();
    let mut left = vec![
        Span::styled(" Tab:mode  ", Style::default().fg(p.dim)),
        Span::styled("/exit  ", Style::default().fg(p.dim)),
        Span::styled("/help  ", Style::default().fg(p.dim)),
    ];
    if auto {
        left.push(Span::styled(
            "⟳auto  ",
            Style::default().fg(p.error).add_modifier(Modifier::BOLD),
        ));
    }
    left.push(Span::styled(status, Style::default().fg(p.dim)));
    let right = Line::from(vec![Span::styled(mode, Style::default().fg(p.accent_bright))]);
    let line = merge_lines(Line::from(left), right, area.width);
    f.render_widget(Paragraph::new(line), area);
}

fn merge_lines<'a>(left: Line<'a>, right: Line<'a>, width: u16) -> Line<'a> {
    let left_w = left.width() as u16;
    let pad = width.saturating_sub(left_w).saturating_sub(2);
    let mut spans = left.spans.clone();
    spans.push(Span::raw(" ".repeat(pad as usize)));
    spans.extend(right.spans);
    Line::from(spans)
}

fn inner(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y.saturating_add(1) as u16,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn header_block(p: Palette) -> Block<'static> {
    Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(p.dim))
}

fn mode_badge_style(_mode: Mode, p: Palette) -> Style {
    Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
}

/// A centered box of the given width/height within `area`.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x.saturating_add(area.width.saturating_sub(w) / 2);
    let y = area.y.saturating_add(area.height.saturating_sub(h) / 2);
    Rect { x, y, width: w, height: h }
}

/// Render the permission dialog overlay (opencode's allow/ask/deny prompt).
fn draw_permission_dialog(
    f: &mut Frame,
    area: Rect,
    req: &(String, String, String),
    p: Palette,
) {
    let (tool, key, input) = req;
    let title = format!(" [ permission: {key} ] ");
    let body = format!(
        "{tool}\n{}\n\napprove once?  (y/1/Enter/Once)\napprove always?(a/2/Tab/Always)\ndeny                 (n/3/Esc/Reject)",
        input
    );
    let n = body.lines().count() as u16 + 2;
    let w = (input.len().max(34).max(title.len().max(20)) + 4) as u16;
    let w = w.min(area.width.saturating_sub(2)).max(20);
    let box_area = centered(area, w, n.saturating_add(2));

    f.render_widget(Clear, box_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.accent).add_modifier(Modifier::BOLD))
        .title(Line::from(vec![Span::styled(
            title,
            Style::default().fg(p.accent_bright).add_modifier(Modifier::BOLD),
        )]));
    let text = Text::from(
        body.lines()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    Line::styled(
                        format!("{line}"),
                        Style::default().fg(p.tool).add_modifier(Modifier::BOLD),
                    )
                } else if i == 1 {
                    Line::styled(format!("{line}"), Style::default().fg(p.assistant))
                } else if line.starts_with("approve once")
                    || line.starts_with("approve always")
                    || line.starts_with("deny")
                {
                    Line::styled(format!("{line}"), Style::default().fg(p.user))
                } else {
                    Line::styled(format!("{line}"), Style::default().fg(p.dim))
                }
            })
            .collect::<Vec<_>>(),
    );
    f.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: false }),
        box_area,
    );
}

/// Render the command palette overlay (opencode `ctrl+p`).
fn draw_palette(f: &mut Frame, area: Rect, sel: usize, items: &[&str], p: Palette) {
    let h = (items.len() as u16 + 2).min(area.height.saturating_sub(2));
    let w = items.iter().map(|s| s.len() as u16).max().unwrap_or(10) + 4;
    let w = w.min(area.width.saturating_sub(2)).max(20);
    let box_area = centered(area, w, h);

    f.render_widget(Clear, box_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.accent))
        .title(Line::from(vec![Span::styled(
            " commands ",
            Style::default().fg(p.accent_bright).add_modifier(Modifier::BOLD),
        )]));
    let mut lines = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let prefix = if i == sel { "› " } else { "  " };
        let line = if i == sel {
            Line::styled(
                format!("{prefix}{item}"),
                Style::default().fg(p.accent_bright).add_modifier(Modifier::BOLD),
            )
        } else {
            Line::styled(format!("{prefix}{item}"), Style::default().fg(p.assistant))
        };
        lines.push(line);
    }
    f.render_widget(Paragraph::new(Text::from(lines)).block(block), box_area);
}