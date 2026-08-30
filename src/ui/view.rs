//! Rendering — maps shared state + mode to widgets styled after opencode.
//!
//! The screen is mirrored from opencode's session view
//! (`packages/tui/src/routes/session/index.tsx` + `component/prompt/index.tsx`):
//!
//! ```text
//! ┌──────────────────────────────────────────┬─────────────┐
//! │  ┃   user message (panel ribbon)         │             │
//! │ ┌┃  ┌┃                                  │  sidebar     │
//! │ │┃  │┃   assistant text / tool blocks    │  (width 42)  │
//! │  ┃                                       │             │
//! │  messages (scrollbox, flexGrow)          │             │
//! │                                          │             │
//! │ ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀  prompt curve            │             │
//! │ directory       ctrl+p commands (status) │             │
//! └──────────────────────────────────────────┴─────────────┘
//! ```
//!
//! The prompt box carries an accent left border (`┃`) over an element
//! background: a blank pad row, the input line, a blank gap, then a meta row
//! (`Mode[ auto] · model provider` + version), then the `╹▀▀` curve row, then
//! the status/footer row.

use super::app::App;
use crate::core::message::{Role, ToolCall, ToolResult};
use crate::theme::Palette;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Sidebar width (opencode `width={42}`).
const SIDEBAR_W: u16 = 42;
/// Prompt zone height: 4 for the bordered box (blank, input, blank, meta),
/// 1 for the `╹▀` curve row, 1 for the status row.
const PROMPT_ZONE_H: u16 = 6;

pub fn draw(f: &mut Frame, app: &App) {
    let palette = Palette::for_mode(app.mode);
    let base = f.area();

    // opencode rides the session `Footer` on its own row at the very bottom,
    // spanning the full width (under both the messages column and the sidebar).
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(base);
    let content_area = vert[0];
    let footer_area = vert[1];

    // Content row: messages column + optional right sidebar.
    let (main, sidebar) = if app.sidebar_open() {
        let c = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(30), Constraint::Length(SIDEBAR_W)])
            .split(content_area);
        (c[0], Some(c[1]))
    } else {
        (content_area, None)
    };

    // opencode `paddingBottom={1} paddingLeft={2} paddingRight={2}`.
    let body = main.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 0,
    });

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(PROMPT_ZONE_H)])
        .split(body);

    draw_messages(f, rows[0], app, palette);
    draw_prompt_zone(f, rows[1], app, palette);

    if let Some(sb) = sidebar {
        draw_sidebar(f, sb, app, palette);
    }
    draw_footer(f, footer_area, palette);

    // Overlays layered on top of the base layout.
    if app.perm_pending() {
        if let Some(req) = app.permission_request() {
            draw_permission_dialog(f, base, &req, palette);
        }
    } else if let Some((sel, items)) = app.palette() {
        draw_palette(f, base, sel, items, palette);
    }
}

/// The prompt box + its `╹▀` curve + the status row.
fn draw_prompt_zone(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    draw_prompt_box(f, rows[0], app, p);
    draw_prompt_curve(f, rows[1], p);
    draw_status(f, rows[2], app, p);
}

/// The accent-left-bordered element pane holding input + meta row.
fn draw_prompt_box(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let text_area = area.inner(ratatui::layout::Margin {
        horizontal: 2, // 1 for `┃` border + 1 extra; opencode paddingLeft2
        vertical: 0,
    });

    // Row 0: blank padding. Row 1: input line (placeholder when empty).
    let input_line = if app.input.is_empty() {
        Line::from(vec![Span::styled(
            "What can OpenSauce build for you?",
            Style::default().fg(p.dim),
        )])
    } else {
        Line::from(vec![Span::styled(app.input.as_str(), Style::default().fg(p.text))])
    };

    // Row 3: meta — `Mode[ auto] · model provider` left, version right.
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
        text_area.width.saturating_sub(2),
    );

    // Left border `┃` in the mode accent.
    f.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(p.accent)),
        area,
    );

    let mut y = text_area.y;
    for line in [Line::raw(""), input_line, Line::raw(""), meta_line] {
        f.render_widget(
            Paragraph::new(line).style(Style::default().bg(p.element)),
            Rect { x: text_area.x, y, width: text_area.width, height: 1 },
        );
        y += 1;
    }

    // Cursor right after the typed text on the input line.
    let x = text_area.x.saturating_add(app.input.chars().count() as u16);
    f.set_cursor_position(Position::new(
        x.min(text_area.right().saturating_sub(1)),
        text_area.y + 1,
    ));
}

/// The box `height={1}` below the prompt: left `╹` (accent) with an upper-half
/// block fill (`▀`) in the element colour — opencode's rounded-bottom edge.
fn draw_prompt_curve(f: &mut Frame, area: Rect, p: Palette) {
    let mut spans: Vec<Span> = vec![Span::styled("╹", Style::default().fg(p.accent))];
    let n = area.width.saturating_sub(1).saturating_sub(2);
    if n > 0 {
        spans.push(Span::styled(
            "▀".repeat(n as usize),
            Style::default().fg(p.element).bg(p.bg),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(p.bg)), area);
}

/// The prompt status row (opencode `status()`): only present while an agent
/// is active — a spinner + message on the left, `esc interrupt` on the right.
/// Idle renders as blank background.
fn draw_status(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    if !app.shared.is_streaming() {
        f.render_widget(Paragraph::new(Line::raw("")).style(Style::default().bg(p.bg)), area);
        return;
    }
    let left_spans = vec![
        Span::styled(" ", Style::default().fg(p.dim)),
        Span::styled("▸ ", Style::default().fg(p.accent)),
        Span::styled("thinking", Style::default().fg(p.warning)),
    ];
    let right_spans = vec![
        Span::styled("esc", Style::default().fg(p.text)),
        Span::styled(" interrupt", Style::default().fg(p.dim)),
    ];
    let line = merge_lines(Line::from(left_spans), Line::from(right_spans), area.width.saturating_sub(2));
    f.render_widget(Paragraph::new(line).style(Style::default().bg(p.bg)), area);
}

/// The full-width bottom footer row (opencode `Footer`): directory left, and
/// `△ n Permission` / `• n LSP ⊙ n MCP /status` right.
fn draw_footer(f: &mut Frame, area: Rect, p: Palette) {
    let dir = std::env::current_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|_| ".".into());
    let left = Line::from(vec![Span::styled(dir, Style::default().fg(p.dim))]);

    let right_spans = vec![
        Span::styled("• ", Style::default().fg(p.success)),
        Span::styled("0 LSP", Style::default().fg(p.text)),
        Span::styled("  ", Style::default().fg(p.dim)),
        Span::styled("⊙ ", Style::default().fg(p.success)),
        Span::styled("0 MCP", Style::default().fg(p.text)),
        Span::styled("  ", Style::default().fg(p.dim)),
        Span::styled("/status", Style::default().fg(p.dim)),
    ];
    let right = Line::from(right_spans);
    let line = merge_lines(left, right, area.width.saturating_sub(2));
    f.render_widget(Paragraph::new(line).style(Style::default().bg(p.bg)), area);
}

fn draw_messages(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let conv = app.shared.lock_conv();
    let turns = conv.turns();
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::raw("")); // opener: opencode's leading `<box height={1} />`

    let last_user = turns.iter().rposition(|m| m.role == Role::User);
    let mut user_idx = 0;

    for m in turns.iter() {
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
                let queued = last_user == Some(user_idx) && app.shared.is_streaming();
                user_ribbon(&mut lines, p, content, queued);
                user_idx += 1;
            }
            Role::Assistant => {
                // Tool-only assistant turns render each tool call as a row
                // plus the results (blocked out below via Tool-role messages).
                if !m.tool_calls.is_empty() {
                    for tc in &m.tool_calls {
                        lines.push(inline_tool(tc, p));
                    }
                    lines.push(Line::raw(""));
                }
                if let Some(c) = &m.content {
                    if !c.trim().is_empty() {
                        lines.push(Line::raw(""));
                        for l in c.lines() {
                            lines.push(Line::from(vec![
                                Span::raw("   "),
                                Span::styled(l.to_string(), Style::default().fg(p.markdown_text)),
                            ]));
                        }
                    }
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
            for l in pending.lines() {
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(l.to_string(), Style::default().fg(p.markdown_text)),
                ]));
            }
            assistant_footer(&mut lines, app, p);
        }
        lines.push(Line::raw(""));
    } else if let Some(last) = turns.iter().rfind(|m| m.role == Role::Assistant) {
        if last.tool_calls.is_empty() && last.content.as_deref().map_or(false, |c| !c.trim().is_empty()) {
            assistant_footer(&mut lines, app, p);
        }
    }

    let total = lines.len();
    let view_h = area.height.saturating_sub(1) as usize;
    let scroll = total.saturating_sub(view_h);

    let text = ratatui::text::Text::from(lines);
    let para = Paragraph::new(text)
        .style(Style::default().bg(p.bg))
        .block(Block::default().padding(Padding {
            left: 1,
            right: 1,
            top: 0,
            bottom: 0,
        }))
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    f.render_widget(para, area);
    drop(conv);
}

/// opencode user ribbon: a `backgroundPanel` pane behind a `┃` border in the
/// agent (mode) colour, with `paddingTop/Bottom={1}` and a QUEUED badge while
/// the newest user turn is still pending approval / being answered.
fn user_ribbon(lines: &mut Vec<Line>, p: Palette, content: &str, queued: bool) {
    let edge = p.accent;
    lines.push(Line::from(vec![
        Span::styled("┃", Style::default().fg(edge)),
        Span::styled("  ", Style::default().bg(p.panel)),
    ]));
    for l in content.lines() {
        lines.push(Line::from(vec![
            Span::raw("┃"),
            Span::styled(format!("  {l}"), Style::default().bg(p.panel).fg(p.text)),
        ]));
    }
    if queued {
        lines.push(Line::from(vec![
            Span::styled("┃", Style::default().fg(edge)),
            Span::styled("   ", Style::default().bg(p.panel)),
            Span::styled(" QUEUED ", Style::default().bg(edge).fg(p.bg).add_modifier(Modifier::BOLD)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("┃", Style::default().fg(edge)),
        Span::styled("  ", Style::default().bg(p.panel)),
    ]));
    lines.push(Line::raw(""));
}

/// opencode assistant footer: `▣ Mode · model` (▣ in the agent colour).
fn assistant_footer(lines: &mut Vec<Line>, app: &App, p: Palette) {
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled("▣ ", Style::default().fg(p.accent)),
        Span::styled(app.mode.label(), Style::default().fg(p.text)),
        Span::styled(format!(" · {}", app.model_label()), Style::default().fg(p.dim)),
    ]));
}

/// A single-line tool row (`InlineTool`): icon + label, muted once complete.
fn inline_tool(tc: &ToolCall, p: Palette) -> Line<'static> {
    let (icon, label) = tool_display(&tc.name, &tc.arguments);
    Line::from(vec![
        Span::raw("   "),
        Span::styled(icon.to_string(), Style::default().fg(p.dim)),
        Span::styled(" ".to_string(), Style::default().fg(p.dim)),
        Span::styled(label, Style::default().fg(p.dim)),
    ])
}

/// Tool result block (`BlockTool`): a `┃` border in the background colour over
/// a `backgroundPanel` pane with `# name args` + trimmed output.
fn tool_output(lines: &mut Vec<Line>, tr: &ToolResult, app: &App, p: Palette) {
    let hl = if tr.success { p.success } else { p.error };
    let (icon, _label) = tool_display(&tr.name, &serde_json::Value::Null);
    lines.push(Line::from(vec![
        Span::styled("┃", Style::default().fg(p.bg)),
        Span::styled("  ", Style::default().bg(p.panel)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("┃", Style::default().fg(p.bg)),
        Span::styled("  ", Style::default().bg(p.panel)),
        Span::styled("# ", Style::default().fg(p.primary)),
        Span::styled(tr.name.clone(), Style::default().fg(p.accent_bright)),
        Span::styled(format!(" {icon}"), Style::default().fg(p.dim)),
        Span::styled(if tr.success { " ✓" } else { " ✗" }, Style::default().fg(hl)),
    ]));
    if app.details() {
        for l in tr.content.lines().take(6) {
            lines.push(Line::from(vec![
                Span::styled("┃", Style::default().fg(p.bg)),
                Span::styled("  ", Style::default().bg(p.panel)),
                Span::styled(l.to_string(), Style::default().fg(p.tool)),
            ]));
        }
    }
    lines.push(Line::from(vec![
        Span::styled("┃", Style::default().fg(p.bg)),
        Span::styled("  ", Style::default().bg(p.panel)),
    ]));
    lines.push(Line::raw(""));
}

/// Map a tool name to `(icon, label)` the way opencode's per-tool components
/// display them (`⚙`, `$`, `→`, `←`, `✱`, `%`, `◈`, …).
fn tool_display(name: &str, args: &serde_json::Value) -> (&'static str, String) {
    let args_str = tool_args(args);
    match name {
        "bash" | "run_command" | "shell" => ("$", args_str),
        "write" | "write_file" => ("←", format!("Write {}", short_str(args, "filePath", args_str))),
        "edit" | "apply_patch" => ("→", format!("Edit {}", short_str(args, "filePath", args_str))),
        "read" | "read_file" => ("→", format!("Read {}", short_str(args, "filePath", args_str))),
        "glob" | "list" => {
            let pattern = short_str(args, "pattern", args_str);
            ("✱", format!("Glob \"{pattern}\""))
        }
        "grep" => {
            let pattern = short_str(args, "pattern", args_str);
            ("✱", format!("Grep \"{pattern}\""))
        }
        "webfetch" => ("%", format!("WebFetch {}", short_str(args, "url", args_str))),
        "websearch" => ("◈", format!("WebSearch \"{}\"", short_str(args, "query", args_str))),
        "task" => ("│", format!("Task: {args_str}")),
        "todowrite" | "todo_write" => ("☐", "TodoWrite".to_string()),
        "skill" => ("◆", format!("Skill {args_str}")),
        other => ("⚙", format!("{other} {args_str}")),
    }
}

fn short_str(args: &serde_json::Value, key: &str, fallback: String) -> String {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or(fallback)
        .replace('"', "")
}

/// Compact human form of tool call arguments.
fn tool_args(v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::Object(map) => map
            .iter()
            .filter(|(k, _)| *k != "content")
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

/// Right sidebar (opencode `width=42`, `backgroundPanel`), with a bold title
/// and the `• OpenCode version` footer.
fn draw_sidebar(f: &mut Frame, area: Rect, app: &App, p: Palette) {
    let title = {
        let conv = app.shared.lock_conv();
        conv.title.clone()
    };

    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });

    let body: Vec<Line> = vec![
        Line::from(vec![Span::styled(
            title,
            Style::default().fg(p.text).add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled("  new session", Style::default().fg(p.dim))]),
        Line::raw(""),
        Line::from(vec![Span::styled("Getting started", Style::default().fg(p.text))]),
        Line::from(vec![Span::styled(
            "  • ask questions, run tools",
            Style::default().fg(p.dim),
        )]),
    ];

    let para = Paragraph::new(body).style(Style::default().bg(p.panel));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    f.render_widget(para, rows[0]);

    let foot = Line::from(vec![
        Span::styled("• ", Style::default().fg(p.success)),
        Span::styled("Open", Style::default().fg(p.dim).add_modifier(Modifier::BOLD)),
        Span::styled("Code", Style::default().fg(p.text).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {VERSION}"), Style::default().fg(p.dim)),
    ]);
    f.render_widget(Paragraph::new(foot).style(Style::default().bg(p.panel)), rows[1]);
}

fn merge_lines(left: Line<'static>, right: Line<'static>, width: u16) -> Line<'static> {
    let left_w = left.width() as u16;
    let right_w = right.width() as u16;
    let avail = width.saturating_sub(left_w).saturating_sub(right_w);
    let pad = avail.max(1);
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

///// Render the permission dialog overlay (opencode allow/ask/deny prompt).
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