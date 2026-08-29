//! Rendering — maps shared state + mode to widgets styled after opencode.
//!
//! The layout mirrors `opencode`'s session view:
//! - messages scroll area (user text on a `backgroundPanel` ribbon with a
//!   colored left border; assistant text indented on the background; tool
//!   calls as `⚙ name` rows),
//! - a bottom prompt box (`backgroundElement` with a left border: input line,
//!   then an agent / model / provider meta row),
//! - a thin status line (`esc interrupt` while streaming),
//! - an optional right sidebar of sessions.

use super::app::App;
use crate::core::message::{Role, ToolCall, ToolResult};
use crate::theme::Palette;
use crate::APP_NAME;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn draw(f: &mut Frame, app: &App) {
    let palette = Palette::for_mode(app.mode);

    // Optional right sidebar; the message column takes the rest.
    let base = f.area();
    let (main, sidebar) = if app.sidebar_open() {
        let c = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(30), Constraint::Length(28)])
            .split(base);
        (c[0], Some(c[1]))
    } else {
        (base, None)
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),   // messages
            Constraint::Length(4), // prompt zone (outer box + status line)
        ])
        .split(main);

    draw_messages(f, rows[0], app, palette);
    draw_prompt_zone(f, rows[1], app, palette);

    if let Some(sb) = sidebar {
        draw_sidebar(f, sb, palette);
    }

    // Overlays layered on top of the base layout.
    if app.perm_pending() {
        if let Some(req) = app.permission_request() {
            draw_permission_dialog(f, base, &req, palette);
        }
    } else if let Some((sel, items)) = app.palette() {
        draw_palette(f, base, sel, items, palette);
    }
}

/// opencode renders no header bar. The screen is: message list + a bottom
/// prompt zone that owns an accent-left-bordered box (element background,
/// input line, agent/meta row) and a single status line beneath it.
fn draw_prompt_zone(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1)])
        .split(area);

    draw_prompt_box(f, rows[0], app, p);
    draw_status(f, rows[1], app, p);
}

/// The prompt box: a `backgroundElement` pane (paddingL 2, paddingTop 1) with
/// an accent left border. Content rows: blank, input line, agent · model ·
/// provider meta row.
fn draw_prompt_box(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    // opencode: accent left border over an element-backgrounded pane.
    let border = area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 0,
    });
    let text = area.inner(ratatui::layout::Margin {
        horizontal: 3, // 1 accent border + 2 opencode padding
        vertical: 0,
    });

    // Initial blank row = opencode `paddingTop: 1`.
    let input_line = if app.input.is_empty() {
        Line::from(vec![Span::styled(
            "What can OpenSauce build for you?",
            Style::default().fg(p.dim),
        )])
    } else {
        Line::from(vec![Span::styled(app.input.as_str(), Style::default().fg(p.text))])
    };

    // opencode meta row: `agent[ auto] · model provider` + version on the right.
    let mut meta = vec![Span::styled(app.mode.label(), Style::default().fg(p.accent))];
    if app.is_auto() {
        meta.push(Span::styled(" auto", Style::default().fg(p.dim)));
    }
    meta.push(Span::styled(" ·", Style::default().fg(p.dim)));
    meta.push(Span::styled(format!(" {}", app.model_label()), Style::default().fg(p.text)));
    meta.push(Span::styled(format!(" {}", app.provider_label()), Style::default().fg(p.dim)));
    let meta_line = merge_lines(
        Line::from(meta),
        Line::from(vec![Span::styled(format!("v{VERSION}"), Style::default().fg(p.dim))]),
        text.width.saturating_sub(2),
    );

    f.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(p.accent)),
        area,
    );
    // Fill the whole pane (incl. the 2-col padding) with the element background.
    f.render_widget(Paragraph::new("").style(Style::default().bg(p.element)), border);

    let mut y = text.y;
    for line in [Line::raw(""), input_line, meta_line] {
        f.render_widget(
            Paragraph::new(line).style(Style::default().bg(p.element)),
            Rect { x: text.x, y, width: text.width, height: 1 },
        );
        y += 1;
    }

    // Cursor right after the typed text on the input line.
    let x = text.x.saturating_add(app.input.chars().count() as u16);
    f.set_cursor_position(Position::new(
        x.min(text.right().saturating_sub(1)),
        text.y + 1,
    ));
}

/// The prompt-embedded status line: directory on the left, `ctrl+p commands`
/// on the right; `▸ thinking` + `esc interrupt` while streaming.
fn draw_status(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let left_spans;
    let right_spans;
    if app.shared.is_streaming() {
        left_spans = vec![
            Span::styled(" ", Style::default().fg(p.dim)),
            Span::styled("▸ ", Style::default().fg(p.accent)),
            Span::styled("thinking", Style::default().fg(p.warning)),
        ];
        right_spans = vec![
            Span::styled("esc", Style::default().fg(p.text)),
            Span::styled(" interrupt", Style::default().fg(p.dim)),
        ];
    } else {
        let dir = std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|_| ".".into());
        left_spans = vec![Span::styled(format!(" {dir}"), Style::default().fg(p.dim))];
        right_spans = vec![
            Span::styled("ctrl+p", Style::default().fg(p.text)),
            Span::styled(" commands", Style::default().fg(p.dim)),
        ];
    }
    let line = merge_lines(Line::from(left_spans), Line::from(right_spans), area.width.saturating_sub(2));
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(p.bg)),
        area,
    );
}

fn draw_messages(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let conv = app.shared.lock_conv();
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::raw("")); // opener: opencode's leading `<box height={1} />`

    for m in conv.turns() {
        match m.role {
            Role::System => {
                if let Some(c) = &m.content {
                    for l in c.lines() {
                        lines.push(Line::from(vec![Span::styled(
                            format!("  {l}"),
                            Style::default().fg(p.dim),
                        )]));
                    }
                }
            }
            Role::User => {
                let content = m.content.as_deref().unwrap_or("");
                user_ribbon(&mut lines, p, content);
            }
            Role::Assistant => {
                if m.tool_calls.is_empty() {
                    if let Some(c) = &m.content {
                        for l in c.lines() {
                            lines.push(Line::from(vec![
                                Span::raw("   "),
                                Span::styled(l.to_string(), Style::default().fg(p.assistant)),
                            ]));
                        }
                        lines.push(Line::raw(""));
                    }
                } else {
                    // opencode: `⚙ name input` row for each tool call
                    for tc in &m.tool_calls {
                        lines.push(tool_call_row(tc, p));
                    }
                    lines.push(Line::raw(""));
                }
            }
            Role::Tool => {
                if let Some(tr) = &m.tool_result {
                    tool_output(&mut lines, tr, app, p);
                }
            }
        }
    }

    // Live streamed assistant text / placeholder.
    if app.shared.is_streaming() {
        let pending = app.shared.pending();
        if pending.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "   Thinking…",
                Style::default().fg(p.warning).add_modifier(Modifier::DIM),
            )]));
        } else {
            assistant_footer(&mut lines, app, p);
            for l in pending.lines() {
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(l.to_string(), Style::default().fg(p.assistant)),
                ]));
            }
        }
        lines.push(Line::raw(""));
    }

    let total = lines.len();
    let view_h = area.height.saturating_sub(2) as usize;
    let scroll = total.saturating_sub(view_h);

    let text = ratatui::text::Text::from(lines);
    let para = Paragraph::new(text)
        .style(Style::default().bg(p.bg))
        .block(Block::default().padding(Padding {
            left: 2,
            right: 2,
            top: 0,
            bottom: 1,
        }))
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    f.render_widget(para, area);
    drop(conv);
}

/// opencode user ribbon: a colored left border + `backgroundPanel`. The border
/// inherits the session agent colour (build → blue, plan → yellow).
fn user_ribbon(lines: &mut Vec<Line>, p: Palette, content: &str) {
    let edge = p.accent;
    lines.push(Line::from(vec![
        Span::styled(" ", Style::default().bg(edge)),
        Span::styled(" ", Style::default().bg(p.panel)),
    ]));
    for l in content.lines() {
        lines.push(Line::from(vec![
            Span::styled(" ", Style::default().bg(edge)),
            Span::styled(format!("  {l}"), Style::default().bg(p.panel).fg(p.text)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(" ", Style::default().bg(edge)),
        Span::styled(" ", Style::default().bg(p.panel)),
    ]));
    lines.push(Line::raw(""));
}

/// opencode assistant footer: `▣ Build · model · duration`.
fn assistant_footer(lines: &mut Vec<Line>, app: &App, p: Palette) {
    lines.push(Line::from(vec![
        Span::styled(" ▣ ", Style::default().fg(p.accent)),
        Span::styled(app.mode.label(), Style::default().fg(p.text)),
        Span::styled(format!(" · {}", app.model_label()), Style::default().fg(p.dim)),
        Span::styled(format!(" · {}", app.provider_label()), Style::default().fg(p.dim)),
        Span::styled(" · generating…", Style::default().fg(p.dim)),
    ]));
}

/// `⚙ name <args>` inline tool row.
fn tool_call_row(tc: &ToolCall, p: Palette) -> Line<'static> {
    let mut spans = vec![
        Span::raw("   "),
        Span::styled("⚙ ", Style::default().fg(p.primary)),
        Span::styled(tc.name.clone(), Style::default().fg(p.text)),
    ];
    let args = tool_args(&tc.arguments);
    if !args.is_empty() {
        spans.push(Span::styled(format!("  {args}"), Style::default().fg(p.dim)));
    }
    Line::from(spans)
}

/// Tool result block: `# name` + trimmed preview (when `details`).
fn tool_output(lines: &mut Vec<Line>, tr: &ToolResult, app: &App, p: Palette) {
    let hl = if tr.success { p.success } else { p.error };
    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled("# ", Style::default().fg(p.primary)),
        Span::styled(tr.name.clone(), Style::default().fg(p.accent_bright)),
        Span::styled(if tr.success { " ✓" } else { " ✗" }, Style::default().fg(hl)),
    ]));
    if app.details() {
        for l in tr.content.lines().take(6) {
            lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled(l.to_string(), Style::default().fg(p.tool)),
            ]));
        }
    }
    lines.push(Line::raw(""));
}

/// Compact human form of tool call arguments.
fn tool_args(v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::Object(map) => map
            .iter()
            .take(3)
            .map(|(k, val)| format!("{k}={}", short(val)))
            .collect::<Vec<_>>()
            .join("  "),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn short(v: &serde_json::Value) -> String {
    let s = match v {
        serde_json::Value::String(x) => x.clone(),
        other => other.to_string(),
    };
    let mut c: Vec<char> = s.chars().collect();
    if c.len() > 40 {
        c.truncate(40);
        format!("{}…", c.into_iter().collect::<String>())
    } else {
        s
    }
}

fn draw_sidebar(f: &mut Frame, area: Rect, p: Palette) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    let mut inner = Vec::new();
    inner.push(Line::from(vec![
        Span::styled("● ", Style::default().fg(p.success)),
        Span::styled("Getting started", Style::default().fg(p.text)),
    ]));
    let dir = std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_else(|_| ".".into());
    inner.push(Line::from(vec![Span::styled(
        format!("  {dir}"),
        Style::default().fg(p.dim),
    )]));
    inner.push(Line::from(vec![Span::styled(
        "  ctrl+x  toggles this panel",
        Style::default().fg(p.dim),
    )]));

    let body = Paragraph::new(inner)
        .style(Style::default().bg(p.panel))
        .block(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(p.border))
                .title(Line::from(vec![Span::styled(
                    " Sessions ",
                    Style::default().fg(p.accent_bright).add_modifier(Modifier::BOLD),
                )])),
        );
    f.render_widget(body, rows[0]);

    let foot = Line::from(vec![
        Span::styled("⚡ ", Style::default().fg(p.primary)),
        Span::styled(format!("{APP_NAME} v{VERSION}"), Style::default().fg(p.text)),
        Span::styled("  Powered By Vexify.", Style::default().fg(p.dim)),
    ]);
    f.render_widget(Paragraph::new(foot).style(Style::default().bg(p.panel)), rows[1]);
}

fn merge_lines(left: Line<'static>, right: Line<'static>, width: u16) -> Line<'static> {
    let left_w = left.width() as u16;
    let pad = width.saturating_sub(left_w).saturating_sub(2).max(1);
    let mut spans = left.spans.clone();
    spans.push(Span::raw(" ".repeat(pad as usize)));
    spans.extend(right.spans);
    Line::from(spans)
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x.saturating_add(area.width.saturating_sub(w) / 2);
    let y = area.y.saturating_add(area.height.saturating_sub(h) / 2);
    Rect { x, y, width: w, height: h }
}

/// Render the permission dialog overlay (opencode allow/ask/deny prompt).
fn draw_permission_dialog(f: &mut Frame, area: Rect, req: &(String, String, String), p: Palette) {
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
    let lines: Vec<Line> = body
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                Line::styled(
                    format!("{line}"),
                    Style::default().fg(p.accent_bright).add_modifier(Modifier::BOLD),
                )
            } else if i == 1 {
                Line::styled(format!("{line}"), Style::default().fg(p.text))
            } else if line.starts_with("approve once")
                || line.starts_with("approve always")
                || line.starts_with("deny")
            {
                Line::styled(format!("{line}"), Style::default().fg(p.user))
            } else {
                Line::styled(format!("{line}"), Style::default().fg(p.dim))
            }
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(p.bg))
            .block(block)
            .wrap(Wrap { trim: false }),
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
            Line::styled(format!("{prefix}{item}"), Style::default().fg(p.text))
        };
        lines.push(line);
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(p.bg)).block(block),
        box_area,
    );
}