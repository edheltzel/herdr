use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use tui_term::widget::PseudoTerminal;

use crate::app::state::{ToastKind, ToastNotification};
use crate::app::{AppState, Mode};
use crate::detect::AgentState;
use crate::layout::PaneInfo;

const COLLAPSED_WIDTH: u16 = 4; // num + space + dot + separator
const MIN_SIDEBAR_WIDTH: u16 = 16;
const MAX_SIDEBAR_WIDTH: u16 = 36;

/// Compute view geometry and reconcile pane sizes.
/// Called before render to separate mutation from drawing.
pub fn compute_view(app: &mut AppState, area: Rect) {
    let sidebar_w = if app.sidebar_collapsed {
        COLLAPSED_WIDTH
    } else {
        compute_sidebar_width(app)
    };

    let [sidebar_area, main_area] =
        Layout::horizontal([Constraint::Length(sidebar_w), Constraint::Min(1)]).areas(area);

    let terminal_area = main_area;

    // Compute split borders
    let split_borders = app
        .active
        .and_then(|i| app.workspaces.get(i))
        .map(|ws| ws.layout.splits(terminal_area))
        .unwrap_or_default();

    // Compute pane layout + reconcile sizes
    let pane_infos = compute_pane_infos(app, terminal_area);

    app.view = crate::app::ViewState {
        sidebar_rect: sidebar_area,
        terminal_area,
        pane_infos,
        split_borders,
    };
}

/// Render the UI — reads AppState but does not mutate it.
pub fn render(app: &AppState, frame: &mut Frame) {
    let sidebar_area = app.view.sidebar_rect;
    let terminal_area = app.view.terminal_area;

    if app.sidebar_collapsed {
        render_sidebar_collapsed(app, frame, sidebar_area);
    } else {
        render_sidebar(app, frame, sidebar_area);
    }
    render_panes(app, frame, terminal_area);

    match app.mode {
        Mode::Onboarding => render_onboarding_overlay(app, frame, frame.area()),
        Mode::Navigate => render_navigate_overlay(app, frame, terminal_area),
        Mode::Resize => render_resize_overlay(app, frame, terminal_area),
        Mode::ConfirmClose => render_confirm_close_overlay(app, frame, terminal_area),
        Mode::ContextMenu => {
            render_navigate_overlay(app, frame, terminal_area);
            render_context_menu(app, frame);
        }
        Mode::RenameSession => {}
        Mode::Terminal => {}
    }

    // Notifications (rendered on top of everything)
    if let Some(version) = &app.update_available {
        if !app.update_dismissed {
            render_update_notification(frame, terminal_area, version, app.accent);
        }
    }
    let has_config_diagnostic = app.config_diagnostic.is_some();
    if let Some(message) = &app.config_diagnostic {
        render_config_diagnostic(frame, terminal_area, message);
    }
    if let Some(toast) = &app.toast {
        render_toast_notification(frame, terminal_area, toast, has_config_diagnostic);
    }
}

/// Compute pane layout info and resize pane runtimes to match.
fn compute_pane_infos(app: &AppState, area: Rect) -> Vec<PaneInfo> {
    let Some(ws_idx) = app.active else {
        return Vec::new();
    };
    let Some(ws) = app.workspaces.get(ws_idx) else {
        return Vec::new();
    };

    if ws.zoomed {
        let focused_id = ws.layout.focused();
        if let Some(rt) = ws.runtimes.get(&focused_id) {
            rt.resize(area.height, area.width);
        }
        return vec![PaneInfo {
            id: focused_id,
            rect: area,
            inner_rect: area,
            is_focused: true,
        }];
    }

    let multi_pane = ws.layout.pane_count() > 1;
    let terminal_active = app.mode == Mode::Terminal;
    let mut pane_infos = ws.layout.panes(area);

    for info in &mut pane_infos {
        let inner = if multi_pane {
            let border_set = if info.is_focused && terminal_active {
                ratatui::symbols::border::THICK
            } else {
                ratatui::symbols::border::PLAIN
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_set(border_set);
            block.inner(info.rect)
        } else {
            area
        };
        info.inner_rect = inner;

        if let Some(rt) = ws.runtimes.get(&info.id) {
            rt.resize(inner.height, inner.width);
        }
    }

    pane_infos
}

/// Auto-scale sidebar width based on workspace identity + agent summary.
fn compute_sidebar_width(app: &AppState) -> u16 {
    if app.workspaces.is_empty() {
        return app.sidebar_width;
    }
    let max_line = app
        .workspaces
        .iter()
        .enumerate()
        .map(|(i, ws)| {
            let name_len = ws.display_name().len();
            let number_len = (i + 1).to_string().len();
            let pane_count = if ws.layout.pane_count() > 1 {
                ws.layout.pane_count()
            } else {
                0
            };
            let line1 = 5 + name_len + number_len + pane_count; // marker + number + name + spaces + pane dots
            let line2 = ws.agent_summary().map(|s| s.len() + 2).unwrap_or(0);
            line1.max(line2)
        })
        .max()
        .unwrap_or(12);
    ((max_line as u16) + 2).clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)
}

/// Collapsed sidebar: pure glance mode.
fn render_sidebar_collapsed(app: &AppState, frame: &mut Frame, area: Rect) {
    let is_navigating = matches!(
        app.mode,
        Mode::Navigate
            | Mode::RenameSession
            | Mode::Resize
            | Mode::ConfirmClose
            | Mode::ContextMenu
    );

    let sep_style = if is_navigating {
        Style::default().fg(app.accent)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let sep_x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(sep_x, y)].set_symbol("│");
        buf[(sep_x, y)].set_style(sep_style);
    }

    let content_w = area.width.saturating_sub(1);
    let bottom_y = area.y + area.height.saturating_sub(1);

    for (i, ws) in app.workspaces.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= bottom_y {
            break;
        }
        let (agg_state, agg_seen) = ws.aggregate_state();
        let (icon, icon_style) = state_icon_style(agg_state, agg_seen);
        let is_selected = i == app.selected && is_navigating;
        let row_style = if is_selected {
            Style::default().bg(app.accent).fg(Color::Black)
        } else {
            Style::default()
        };
        let num_style = if is_selected {
            row_style
        } else {
            Style::default().fg(Color::DarkGray)
        };

        if is_selected {
            let buf = frame.buffer_mut();
            for x in area.x..area.x + content_w {
                buf[(x, y)].set_style(row_style);
            }
        }

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{}", i + 1), num_style),
                Span::styled(" ", row_style),
                Span::styled(
                    icon,
                    if is_selected {
                        icon_style.bg(app.accent)
                    } else {
                        icon_style
                    },
                ),
            ])),
            Rect::new(area.x, y, content_w, 1),
        );
    }

    render_sidebar_toggle(frame, area, true);
}

fn render_sidebar(app: &AppState, frame: &mut Frame, area: Rect) {
    let is_navigating = matches!(
        app.mode,
        Mode::Navigate
            | Mode::RenameSession
            | Mode::Resize
            | Mode::ConfirmClose
            | Mode::ContextMenu
    );
    let highlight = Style::default().bg(app.accent).fg(Color::Black);
    let sep_style = if is_navigating {
        Style::default().fg(app.accent)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let sep_x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(sep_x, y)].set_symbol("│");
        buf[(sep_x, y)].set_style(sep_style);
    }

    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    let bottom_y = content.y + content.height.saturating_sub(1);
    let new_row_y = bottom_y.saturating_sub(1);
    let mut row_y = content.y;

    for (i, ws) in app.workspaces.iter().enumerate() {
        if row_y + 1 >= new_row_y {
            break;
        }
        let selected = i == app.selected && is_navigating;
        let marker = if Some(i) == app.active { "▸" } else { " " };
        let (agg_state, agg_seen) = ws.aggregate_state();
        let (icon, icon_style) = state_icon_style(agg_state, agg_seen);
        let text_style = if selected {
            highlight
        } else {
            Style::default()
        };
        let dim_style = if selected {
            highlight
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut line1 = vec![
            Span::styled(marker, text_style),
            Span::styled(format!("{} ", i + 1), dim_style),
            Span::styled(ws.display_name(), text_style.add_modifier(Modifier::BOLD)),
            Span::styled("  ", dim_style),
        ];

        if ws.layout.pane_count() == 1 {
            line1.push(Span::styled(
                icon,
                if selected {
                    icon_style.bg(app.accent)
                } else {
                    icon_style
                },
            ));
        } else {
            for (pane_state, pane_seen) in ws.pane_states() {
                let (pane_icon, pane_style) = state_icon_style(pane_state, pane_seen);
                line1.push(Span::styled(
                    pane_icon,
                    if selected {
                        pane_style.bg(app.accent)
                    } else {
                        pane_style
                    },
                ));
            }
        }

        if selected {
            let buf = frame.buffer_mut();
            for y in row_y..=row_y + 1 {
                for x in content.x..content.x + content.width {
                    buf[(x, y)].set_style(highlight);
                }
            }
        }

        frame.render_widget(
            Paragraph::new(Line::from(line1)),
            Rect::new(content.x, row_y, content.width, 1),
        );

        if app.mode == Mode::RenameSession && i == app.selected {
            let text = format!("  {}\u{2588}", app.name_input);
            frame.render_widget(Clear, Rect::new(content.x, row_y + 1, content.width, 1));
            frame.render_widget(
                Paragraph::new(text).style(Style::default().fg(Color::Yellow)),
                Rect::new(content.x, row_y + 1, content.width, 1),
            );
        } else if let Some(summary) = ws.agent_summary() {
            let line2 = Line::from(vec![
                Span::styled("  ", dim_style),
                Span::styled(summary, dim_style),
            ]);
            frame.render_widget(
                Paragraph::new(line2),
                Rect::new(content.x, row_y + 1, content.width, 1),
            );
        }

        row_y += 2;
    }

    frame.render_widget(
        Paragraph::new(Span::styled("new", Style::default().fg(Color::DarkGray))),
        Rect::new(content.x, new_row_y, content.width, 1),
    );
    render_sidebar_toggle(frame, area, false);
}

fn render_sidebar_toggle(frame: &mut Frame, area: Rect, collapsed: bool) {
    let bottom_y = area.y + area.height.saturating_sub(1);
    let content_w = area.width.saturating_sub(1); // exclude separator
    if content_w == 0 || area.height == 0 {
        return;
    }
    let icon = if collapsed { "»" } else { "«" };
    // Center the icon in the content area
    let x = area.x + content_w / 2;
    let toggle_area = Rect::new(x, bottom_y, 1, 1);
    frame.render_widget(
        Paragraph::new(Span::styled(icon, Style::default().fg(Color::DarkGray))),
        toggle_area,
    );
}

fn render_panes(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(ws_idx) = app.active else {
        render_empty(frame, area, app.accent);
        return;
    };
    let Some(ws) = app.workspaces.get(ws_idx) else {
        render_empty(frame, area, app.accent);
        return;
    };

    let multi_pane = ws.layout.pane_count() > 1;
    let terminal_active = app.mode == Mode::Terminal;

    for info in &app.view.pane_infos {
        if let Some(rt) = ws.runtimes.get(&info.id) {
            // Draw borders for multi-pane layouts
            if multi_pane {
                let (border_style, border_set) = if info.is_focused && terminal_active {
                    (
                        Style::default().fg(app.accent),
                        ratatui::symbols::border::THICK,
                    )
                } else if info.is_focused {
                    (
                        Style::default().fg(app.accent),
                        ratatui::symbols::border::PLAIN,
                    )
                } else {
                    (
                        Style::default().fg(Color::DarkGray),
                        ratatui::symbols::border::PLAIN,
                    )
                };

                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .border_set(border_set);
                frame.render_widget(block, info.rect);
            }

            // Draw terminal content
            if let Ok(parser) = rt.parser.read() {
                let pt = PseudoTerminal::new(parser.screen());
                frame.render_widget(pt, info.inner_rect);
            }

            // Dim unfocused panes only in navigate mode
            let should_dim = !info.is_focused && multi_pane && !terminal_active;
            if should_dim {
                let inner = info.inner_rect;
                let buf = frame.buffer_mut();
                for y in inner.y..inner.y + inner.height {
                    for x in inner.x..inner.x + inner.width {
                        let cell = &mut buf[(x, y)];
                        let style = cell.style();
                        let fg = style.fg.unwrap_or(Color::White);
                        let dimmed_fg = dim_color(fg);
                        cell.set_style(style.fg(dimmed_fg));
                    }
                }
            }

            // Selection highlight
            render_selection_highlight(&app.selection, frame, info.id, info.inner_rect);
        }
    }
}

/// Render selection highlight for a pane by inverting fg/bg colors.
/// Reduce a color's brightness by blending it toward black.
fn dim_color(color: Color) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(r / 3, g / 3, b / 3),
        Color::White => Color::DarkGray,
        Color::Gray => Color::DarkGray,
        Color::DarkGray => Color::Rgb(30, 30, 30),
        Color::Red => Color::Rgb(60, 0, 0),
        Color::Green => Color::Rgb(0, 60, 0),
        Color::Yellow => Color::Rgb(60, 60, 0),
        Color::Blue => Color::Rgb(0, 0, 60),
        Color::Magenta => Color::Rgb(60, 0, 60),
        Color::Cyan => Color::Rgb(0, 60, 60),
        Color::LightRed => Color::Rgb(80, 30, 30),
        Color::LightGreen => Color::Rgb(30, 80, 30),
        Color::LightYellow => Color::Rgb(80, 80, 30),
        Color::LightBlue => Color::Rgb(30, 30, 80),
        Color::LightMagenta => Color::Rgb(80, 30, 80),
        Color::LightCyan => Color::Rgb(30, 80, 80),
        // Indexed colors and others: just use DIM modifier as fallback
        _ => Color::DarkGray,
    }
}

fn render_selection_highlight(
    selection: &Option<crate::selection::Selection>,
    frame: &mut Frame,
    pane_id: crate::layout::PaneId,
    inner: Rect,
) {
    if let Some(sel) = selection {
        if sel.is_visible() && sel.pane_id == pane_id {
            let buf = frame.buffer_mut();
            for y in 0..inner.height {
                for x in 0..inner.width {
                    if sel.contains(y, x) {
                        let cell = &mut buf[(inner.x + x, inner.y + y)];
                        // Fixed highlight: white text on blue background.
                        // Consistent regardless of the cell's original colors.
                        cell.set_style(
                            Style::default()
                                .fg(Color::White)
                                .bg(Color::Rgb(40, 80, 140)),
                        );
                    }
                }
            }
        }
    }
}

fn render_empty(frame: &mut Frame, area: Rect, accent: Color) {
    let lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "  No active workspace",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Press ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "new",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to create one", Style::default().fg(Color::DarkGray)),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        area,
    );
}

const ONBOARDING_PREFIX_LABEL: &str = "ctrl+b";

fn dim_background(frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut buf[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }
}

fn render_modal_shell(
    frame: &mut Frame,
    area: Rect,
    popup_w: u16,
    popup_h: u16,
    accent: Color,
) -> Option<Rect> {
    let popup_w = popup_w.min(area.width.saturating_sub(4));
    let popup_h = popup_h.min(area.height.saturating_sub(2));
    if popup_w < 4 || popup_h < 4 {
        return None;
    }

    let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(popup_x, popup_y, popup_w, popup_h);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    Some(inner)
}

fn render_modal_header(frame: &mut Frame, area: Rect, title: &str, accent: Color) {
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        )),
        area,
    );
}

fn render_onboarding_overlay(app: &AppState, frame: &mut Frame, area: Rect) {
    dim_background(frame, area);

    match app.onboarding_step {
        0 => render_onboarding_welcome(app, frame, area),
        _ => render_onboarding_notifications(app, frame, area),
    }
}

fn render_onboarding_welcome(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(inner) = render_modal_shell(frame, area, 64, 15, app.accent) else {
        return;
    };
    if inner.height < 10 {
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas::<11>(inner);

    frame.render_widget(
        Paragraph::new("  herdr").style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new("  workspace manager for coding agents")
            .style(Style::default().fg(Color::DarkGray)),
        rows[2],
    );

    let line1 = Line::from(vec![
        Span::styled(
            format!("  {}", ONBOARDING_PREFIX_LABEL),
            Style::default().fg(app.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" navigate ", Style::default().fg(Color::Gray)),
        Span::styled("·", Style::default().fg(Color::DarkGray)),
        Span::styled(" click sidebar ", Style::default().fg(Color::Gray)),
        Span::styled("·", Style::default().fg(Color::DarkGray)),
        Span::styled(" scroll panes", Style::default().fg(Color::Gray)),
    ]);
    frame.render_widget(Paragraph::new(line1), rows[4]);

    let line2 = Line::from(vec![
        Span::styled(
            "  ↑↓",
            Style::default().fg(app.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" switch workspace ", Style::default().fg(Color::Gray)),
        Span::styled("·", Style::default().fg(Color::DarkGray)),
        Span::styled(" drag borders ", Style::default().fg(Color::Gray)),
        Span::styled("·", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " ⇥",
            Style::default().fg(app.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" pane", Style::default().fg(Color::Gray)),
    ]);
    frame.render_widget(Paragraph::new(line2), rows[5]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ● ", Style::default().fg(Color::Red)),
            Span::styled("needs you    ", Style::default().fg(Color::Gray)),
            Span::styled("○ ", Style::default().fg(Color::Yellow)),
            Span::styled("working    ", Style::default().fg(Color::Gray)),
            Span::styled("◌ ", Style::default().fg(Color::DarkGray)),
            Span::styled("no agent", Style::default().fg(Color::Gray)),
        ])),
        rows[7],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  continue  ",
                Style::default()
                    .fg(Color::Black)
                    .bg(app.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   readme has config and more",
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        rows[9],
    );
}

fn render_onboarding_notifications(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(inner) = render_modal_shell(frame, area, 52, 10, app.accent) else {
        return;
    };

    if inner.height < 7 {
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas::<7>(inner);

    render_modal_header(frame, rows[0], "choose notification style", app.accent);
    frame.render_widget(
        Paragraph::new(" herdr can alert you when background work needs attention or finishes.")
            .style(Style::default().fg(Color::Gray)),
        rows[1],
    );

    let options = [
        "quiet        no sound, no visual toasts",
        "visual only  top-right toasts, no sound",
        "sound only   sound alerts, no toasts",
        "both         sound and visual toasts",
    ];

    for (idx, option) in options.iter().enumerate() {
        let selected = idx == app.onboarding_selected;
        let prefix = if selected { "›" } else { " " };
        let style = if selected {
            Style::default().fg(Color::Black).bg(app.accent)
        } else {
            Style::default().fg(Color::White)
        };
        frame.render_widget(
            Paragraph::new(format!(" {prefix} {}. {option}", idx + 1)).style(style),
            rows[idx + 2],
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [ back ] ", Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(
                " [ save ] ",
                Style::default()
                    .fg(Color::Black)
                    .bg(app.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[6],
    );
}

/// Floating overlay for navigate mode — appears at bottom of terminal area.
fn render_navigate_overlay(app: &AppState, frame: &mut Frame, area: Rect) {
    let key = Style::default().fg(app.accent).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let label = Style::default().fg(Color::White);

    let kb = &app.keybinds;
    let line1 = Line::from(vec![
        Span::styled(format!(" {}", kb.new_workspace_label), key),
        Span::styled(" new  ", dim),
        Span::styled(kb.rename_workspace_label.as_str(), key),
        Span::styled(" rename  ", dim),
        Span::styled(kb.close_workspace_label.as_str(), key),
        Span::styled(" close ws  ", dim),
        Span::styled(kb.split_vertical_label.as_str(), key),
        Span::styled(" split│  ", dim),
        Span::styled(kb.split_horizontal_label.as_str(), key),
        Span::styled(" split─  ", dim),
        Span::styled(kb.close_pane_label.as_str(), key),
        Span::styled(" close pane  ", dim),
        Span::styled(kb.fullscreen_label.as_str(), key),
        Span::styled(" fullscreen", dim),
    ]);

    let ws_name = app
        .active
        .and_then(|i| app.workspaces.get(i))
        .map(|ws| ws.display_name())
        .unwrap_or_else(|| "—".to_string());

    let pane_info = app
        .active
        .and_then(|i| app.workspaces.get(i))
        .filter(|ws| ws.layout.pane_count() > 1)
        .map(|ws| {
            let ids = ws.layout.pane_ids();
            let pos = ids
                .iter()
                .position(|id| *id == ws.layout.focused())
                .unwrap_or(0);
            format!(" [{}/{}]", pos + 1, ids.len())
        })
        .unwrap_or_default();

    let mode_style = Style::default()
        .fg(Color::Black)
        .bg(app.accent)
        .add_modifier(Modifier::BOLD);

    let line2 = Line::from(vec![
        Span::styled(" NAVIGATE ", mode_style),
        Span::raw(" "),
        Span::styled(ws_name, label),
        Span::styled(&pane_info, dim),
        Span::raw("  "),
        Span::styled("esc", key),
        Span::styled(" back  ", dim),
        Span::styled("↑↓", key),
        Span::styled(" ws  ", dim),
        Span::styled("⇥", key),
        Span::styled(" pane  ", dim),
        Span::styled(kb.resize_mode_label.as_str(), key),
        Span::styled(" resize  ", dim),
        Span::styled(kb.toggle_sidebar_label.as_str(), key),
        Span::styled(" sidebar  ", dim),
        Span::styled("⏎", key),
        Span::styled(" open  ", dim),
        Span::styled("q", key),
        Span::styled(" quit", dim),
    ]);

    let overlay_height = 2;
    let overlay_y = area.y + area.height.saturating_sub(overlay_height);
    let overlay_area = Rect::new(area.x, overlay_y, area.width, overlay_height);

    // Clear the area behind the overlay
    frame.render_widget(Clear, overlay_area);

    let bg = Style::default().bg(Color::Black);
    let buf = frame.buffer_mut();
    for y in overlay_area.y..overlay_area.y + overlay_area.height {
        for x in overlay_area.x..overlay_area.x + overlay_area.width {
            buf[(x, y)].set_style(bg);
        }
    }

    let [row1, row2] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(overlay_area);
    frame.render_widget(Paragraph::new(line1), row1);
    frame.render_widget(Paragraph::new(line2), row2);
}

/// Floating overlay for resize mode.
fn render_resize_overlay(app: &AppState, frame: &mut Frame, area: Rect) {
    let key = Style::default().fg(app.accent).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);

    let mode_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Magenta)
        .add_modifier(Modifier::BOLD);

    let line = Line::from(vec![
        Span::styled(" RESIZE ", mode_style),
        Span::raw("  "),
        Span::styled("h/l", key),
        Span::styled(" width  ", dim),
        Span::styled("j/k", key),
        Span::styled(" height  ", dim),
        Span::styled("esc", key),
        Span::styled(" done", dim),
    ]);

    let overlay_y = area.y + area.height.saturating_sub(1);
    let overlay_area = Rect::new(area.x, overlay_y, area.width, 1);

    frame.render_widget(Clear, overlay_area);
    let bg = Style::default().bg(Color::Black);
    let buf = frame.buffer_mut();
    for x in overlay_area.x..overlay_area.x + overlay_area.width {
        buf[(x, overlay_y)].set_style(bg);
    }
    frame.render_widget(Paragraph::new(line), overlay_area);
}

/// Centered popup confirmation dialog with dimmed background.
fn render_confirm_close_overlay(app: &AppState, frame: &mut Frame, area: Rect) {
    let ws_name = app
        .workspaces
        .get(app.selected)
        .map(|ws| ws.display_name())
        .unwrap_or_else(|| "?".to_string());
    let pane_count = app
        .workspaces
        .get(app.selected)
        .map(|ws| ws.layout.pane_count())
        .unwrap_or(0);

    let pane_text = if pane_count == 1 {
        "1 pane".to_string()
    } else {
        format!("{pane_count} panes")
    };

    // Dim the entire background
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut buf[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }

    // Centered popup
    let popup_w = 44u16.min(area.width.saturating_sub(4));
    let popup_h = 5u16;
    let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(popup_x, popup_y, popup_w, popup_h);

    let key = Style::default().fg(app.accent).add_modifier(Modifier::BOLD);
    let warn = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);

    let title_line = Line::from(vec![Span::styled(" Close workspace?", warn)]);

    let detail_line = Line::from(vec![
        Span::styled(
            format!(" {ws_name}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" — {pane_text}"), dim),
    ]);

    let action_line = Line::from(vec![
        Span::raw(" "),
        Span::styled("y", key),
        Span::styled("/", dim),
        Span::styled("enter", key),
        Span::styled(" confirm    ", dim),
        Span::styled("any key", key),
        Span::styled(" cancel", dim),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    if inner.height >= 3 {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas::<3>(inner);
        frame.render_widget(Paragraph::new(title_line), rows[0]);
        frame.render_widget(Paragraph::new(detail_line), rows[1]);
        frame.render_widget(Paragraph::new(action_line), rows[2]);
    }
}

/// Right-click context menu popup anchored near the click position.
fn render_context_menu(app: &AppState, frame: &mut Frame) {
    use crate::app::CONTEXT_MENU_ITEMS;

    let Some(menu) = &app.context_menu else {
        return;
    };

    let menu_w = 14u16;
    let menu_h = CONTEXT_MENU_ITEMS.len() as u16 + 2; // +2 for border
    let area = frame.area();

    // Position: try to place below-right of click, clamp to screen
    let x = menu.x.min(area.width.saturating_sub(menu_w));
    let y = menu.y.min(area.height.saturating_sub(menu_h));
    let menu_rect = Rect::new(x, y, menu_w, menu_h);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.accent))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(menu_rect);

    frame.render_widget(Clear, menu_rect);
    frame.render_widget(block, menu_rect);

    let highlight = Style::default()
        .fg(Color::Black)
        .bg(app.accent)
        .add_modifier(Modifier::BOLD);
    let normal = Style::default().fg(Color::White);

    for (i, item) in CONTEXT_MENU_ITEMS.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let style = if i == menu.selected {
            highlight
        } else {
            normal
        };
        let row = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
        frame.render_widget(Paragraph::new(format!(" {item}")).style(style), row);
    }
}

fn render_update_notification(frame: &mut Frame, area: Rect, version: &str, accent: Color) {
    let text = format!(" ✦ herdr v{version} installed — restart to update ");
    let width = text.len() as u16 + 2;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(3);
    let notif_area = Rect::new(x, y, width.min(area.width), 1);

    frame.render_widget(Clear, notif_area);
    frame.render_widget(
        Paragraph::new(Span::styled(
            text,
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        )),
        notif_area,
    );
}

fn render_toast_notification(
    frame: &mut Frame,
    area: Rect,
    toast: &ToastNotification,
    offset_for_warning: bool,
) {
    let dot_color = match toast.kind {
        ToastKind::NeedsAttention => Color::Red,
        ToastKind::Finished => Color::Blue,
    };
    let content_width = (toast.title.len().max(toast.context.len()) as u16) + 4;
    let width = content_width.saturating_add(2).min(area.width);
    let height = 4u16.min(area.height);
    let x = area.x + area.width.saturating_sub(width);
    let y = area.y + if offset_for_warning { 1 } else { 0 };
    let toast_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, toast_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(toast_area);
    frame.render_widget(block, toast_area);

    if inner.height < 2 {
        return;
    }

    let [title_row, context_row] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);

    let title = Line::from(vec![
        Span::styled("●", Style::default().fg(dot_color)),
        Span::raw(" "),
        Span::styled(
            &toast.title,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let context = Line::from(vec![
        Span::styled("  ", Style::default().fg(Color::DarkGray)),
        Span::styled(&toast.context, Style::default().fg(Color::DarkGray)),
    ]);

    frame.render_widget(Paragraph::new(title), title_row);
    frame.render_widget(Paragraph::new(context), context_row);
}

/// Visual badge for a pane's state + seen flag.
///
/// | State              | Icon | Color  |
/// |--------------------|------|--------|
/// | Busy               | ●    | Yellow |
/// | Done (idle+unseen) | ●    | Blue   |
/// | Idle (seen)        | ○    | Green  |
/// | Unknown            | ·    | Gray   |
///
/// Filled dot = needs attention (working, or finished unseen).
/// Hollow dot = nothing to do here.
fn render_config_diagnostic(frame: &mut Frame, area: Rect, message: &str) {
    let text = format!(" config warning: {message} ");
    let width = text.len() as u16 + 2;
    let notif_area = Rect::new(
        area.x + area.width.saturating_sub(width.min(area.width)),
        area.y,
        width.min(area.width),
        1,
    );

    frame.render_widget(Clear, notif_area);
    frame.render_widget(
        Paragraph::new(Span::styled(
            text,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        notif_area,
    );
}

fn state_icon_style(state: AgentState, seen: bool) -> (&'static str, Style) {
    match (state, seen) {
        (AgentState::Waiting, _) => ("●", Style::default().fg(Color::Red)),
        (AgentState::Busy, _) => ("●", Style::default().fg(Color::Yellow)),
        (AgentState::Idle, false) => ("●", Style::default().fg(Color::Blue)), // Done
        (AgentState::Idle, true) => ("○", Style::default().fg(Color::Green)),
        (AgentState::Unknown, _) => ("·", Style::default().fg(Color::DarkGray)),
    }
}

fn _build_hints(items: &[(&str, &str)], key_style: Style, dim_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    spans.push(Span::raw(" "));
    for (i, (k, desc)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", dim_style));
        }
        spans.push(Span::styled(k.to_string(), key_style));
        spans.push(Span::styled(format!(" {desc}"), dim_style));
    }
    spans
}
