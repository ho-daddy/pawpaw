use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

use super::{
    app::{App, PanelSide, Screen},
    dialogs,
    file_editor,
    file_info,
    file_viewer,
    panel,
    process_manager,
    ai_screen,
    system_info,
    advanced_search,
    image_viewer,
    search_result,
    help,
    theme::Theme,
};

const APP_TITLE: &str = concat!("COKACDIR v", env!("CARGO_PKG_VERSION"));

pub fn draw(frame: &mut Frame, app: &mut App) {
    // Clone theme to avoid borrow conflict with mutable app
    let theme = app.theme.clone();
    let area = frame.area();

    // Check if terminal is too large for ratatui buffer
    let frame_area = frame.area();
    if (frame_area.width as u32 * frame_area.height as u32) > 65534 {
        // Terminal too large, show warning message only
        let msg = Paragraph::new("Terminal too large. Please resize smaller.")
            .style(Style::default().fg(theme.message.text).add_modifier(Modifier::BOLD));
        let safe_rect = Rect::new(0, 0, frame_area.width.min(80), 1);
        frame.render_widget(msg, safe_rect);
        return;
    }

    // Fill entire screen with background color first
    let background = Block::default().style(Style::default().bg(theme.palette.bg));
    frame.render_widget(background, area);

    // Clear the entire screen first for full-screen views
    match app.current_screen {
        Screen::AIScreen | Screen::SystemInfo => {
            frame.render_widget(Clear, area);
        }
        _ => {}
    }

    match app.current_screen {
        Screen::DualPanel => draw_dual_panel(frame, app, area, &theme),
        Screen::FileViewer => {
            if app.is_ai_mode() {
                // AI 모드: 뷰어와 AI 화면을 나란히 표시
                draw_viewer_with_ai(frame, app, area, &theme);
            } else if let Some(ref mut state) = app.viewer_state {
                file_viewer::draw(frame, state, area, &theme);
            }
        }
        Screen::FileEditor => {
            if app.is_ai_mode() {
                // AI 모드: 에디터와 AI 화면을 나란히 표시
                draw_editor_with_ai(frame, app, area, &theme);
            } else if let Some(ref mut state) = app.editor_state {
                file_editor::draw(frame, state, area, &theme);
            }
        }
        Screen::FileInfo => file_info::draw(frame, app, area, &theme),
        Screen::ProcessManager => process_manager::draw(frame, app, area, &theme),
        Screen::Help => help::draw(frame, app, area, &theme),
        Screen::AIScreen => {
            if let Some(ref mut state) = app.ai_state {
                ai_screen::draw(frame, state, area, &theme);
            }
        }
        Screen::SystemInfo => {
            system_info::draw(frame, &app.system_info_state, area, &theme);
        }
        Screen::ImageViewer => {
            // 이미지 뷰어는 항상 배경(dual panel) 위에 오버레이로 표시
            // AI 모드여도 draw_dual_panel_background가 AI 패널을 포함해서 그림
            image_viewer::draw(frame, app, area, &theme);
        }
        Screen::SearchResult => {
            search_result::draw(frame, &mut app.search_result_state, area, &theme);
        }
    }

    // Draw advanced search dialog overlay if active
    if app.advanced_search_state.active && app.current_screen == Screen::DualPanel {
        advanced_search::draw(frame, &app.advanced_search_state, area, &theme);
    }

    // Draw dialog overlay on top of everything (모든 화면 위에 다이얼로그 표시)
    if let Some(ref dialog) = app.dialog {
        dialogs::draw_dialog(frame, app, dialog, area, &theme);
    }

    // Update message timer
    if app.message_timer > 0 {
        app.message_timer -= 1;
        if app.message_timer == 0 {
            app.message = None;
        }
    }
}

fn draw_dual_panel(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    // Layout: Panels, Status Bar, Function Bar (no header - saves 1 line)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // Panels
            Constraint::Length(1), // Status bar
            Constraint::Length(1), // Function bar / message
        ])
        .split(area);

    // Dual panels
    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[0]);

    let left_path_str = app.left_panel.path.display().to_string();
    let right_path_str = app.right_panel.path.display().to_string();
    let left_bookmarked = app.settings.bookmarked_path.contains(&left_path_str);
    let right_bookmarked = app.settings.bookmarked_path.contains(&right_path_str);

    // AI 모드일 때 해당 패널에 AI 화면 표시
    let ai_on_left = app.ai_panel_side == Some(PanelSide::Left);
    let ai_on_right = app.ai_panel_side == Some(PanelSide::Right);
    let is_ai_mode = app.is_ai_mode();
    let has_dialog = app.dialog.is_some();
    let active_panel = app.active_panel;

    // 왼쪽 패널: AI 또는 파일 목록
    if ai_on_left {
        if let Some(ref mut state) = app.ai_state {
            let ai_focused = active_panel == PanelSide::Left && !has_dialog;
            ai_screen::draw_with_focus(frame, state, panel_chunks[0], theme, ai_focused);
        }
    } else {
        // AI 모드에서 파일 패널에 포커스: active_panel이 이쪽이고 AI가 반대쪽에 있을 때
        let left_focused = active_panel == PanelSide::Left && !has_dialog && (!is_ai_mode || ai_on_right);
        panel::draw(
            frame,
            &mut app.left_panel,
            panel_chunks[0],
            left_focused,
            left_bookmarked,
            theme,
        );
    }

    // 오른쪽 패널: AI 또는 파일 목록
    if ai_on_right {
        if let Some(ref mut state) = app.ai_state {
            let ai_focused = active_panel == PanelSide::Right && !has_dialog;
            ai_screen::draw_with_focus(frame, state, panel_chunks[1], theme, ai_focused);
        }
    } else {
        let right_focused = active_panel == PanelSide::Right && !has_dialog && (!is_ai_mode || ai_on_left);
        panel::draw(
            frame,
            &mut app.right_panel,
            panel_chunks[1],
            right_focused,
            right_bookmarked,
            theme,
        );
    }

    // Status bar
    draw_status_bar(frame, app, chunks[1], theme);

    // Function bar or message
    draw_function_bar(frame, app, chunks[2], theme);

    // Dialog overlay는 draw() 함수 끝에서 모든 화면 위에 그려짐
}

/// Public function for drawing dual panel background (used by overlay screens)
pub fn draw_dual_panel_background(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    draw_dual_panel(frame, app, area, theme);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let panel = app.active_panel();
    let current_file = panel.current_file();

    let left_text = if let Some(file) = current_file {
        if file.name != ".." {
            format!(
                "{} ({})",
                file.name,
                crate::utils::format::format_size(file.size)
            )
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let selected_count = panel.selected_files.len();
    let total_size: u64 = panel
        .files
        .iter()
        .filter(|f| !f.is_directory)
        .map(|f| f.size)
        .sum();

    let right_text = if selected_count > 0 {
        format!(
            "{} selected, Total: {}",
            selected_count,
            crate::utils::format::format_size(total_size)
        )
    } else {
        format!("Total: {}", crate::utils::format::format_size(total_size))
    };

    let status = Line::from(vec![
        Span::styled(format!(" {} ", left_text), theme.status_bar_style()),
        Span::styled(
            " ".repeat(area.width.saturating_sub(left_text.len() as u16 + right_text.len() as u16 + 4) as usize),
            theme.status_bar_style(),
        ),
        Span::styled(format!(" {} ", right_text), theme.status_bar_style()),
    ]);

    frame.render_widget(Paragraph::new(status).style(theme.status_bar_style()), area);
}

fn draw_function_bar(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    // Show message if present
    if let Some(ref msg) = app.message {
        let message = Paragraph::new(Span::styled(
            format!(" {} ", msg),
            Style::default().fg(theme.message.text).add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(message, area);
        return;
    }

    // 단축키: 첫 글자 강조 스타일
    let mut shortcuts: Vec<(&str, &str)> = vec![
        ("h", "elp "),
        ("i", "nfo "),
        ("e", "dit "),
        ("k", "mkdir "),
        ("m", "kfile "),
        ("x", "del "),
        ("r", "en "),
        ("t", "ar "),
        ("u", "hnd "),
        ("f", "ind "),
        (".", "AI "),
        ("p", "roc "),
        ("Spc", "sel "),
        ("S↑↓", "sel "),
        (";", "ext "),
        ("^a", "ll "),
        ("^c", "py "),
        ("^x", "ut "),
        ("^v/V", "pst "),
        ("nsdy", "sort "),
        ("1", "hom "),
        ("2", "ref "),
        ("'", "mrk "),
    ];

    // macOS only: open in Finder, open in VS Code
    #[cfg(target_os = "macos")]
    {
        shortcuts.push(("o", "pen "));
        shortcuts.push(("c", "ode "));
    }

    shortcuts.push(("`", "set "));
    shortcuts.push(("q", "uit"));

    let mut spans = Vec::new();
    for (key, rest) in &shortcuts {
        spans.push(Span::styled(*key, Style::default().fg(theme.function_bar.key)));
        spans.push(Span::styled(*rest, Style::default().fg(theme.function_bar.label)));
    }

    // Calculate shortcuts width and add padding + version
    let shortcuts_width: usize = shortcuts.iter().map(|(k, r)| k.len() + r.len()).sum();
    let version_text = format!(" {}", APP_TITLE);
    let padding_width = (area.width as usize).saturating_sub(shortcuts_width + version_text.len());

    spans.push(Span::styled(" ".repeat(padding_width), theme.dim_style()));
    spans.push(Span::styled(version_text, theme.dim_style()));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// AI 모드에서 에디터와 AI 화면을 나란히 표시
fn draw_editor_with_ai(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    // Layout: Panels, Status Bar, Function Bar (same as draw_dual_panel)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // Panels
            Constraint::Length(1), // Status bar
            Constraint::Length(1), // Function bar
        ])
        .split(area);

    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[0]);

    let ai_on_left = app.ai_panel_side == Some(PanelSide::Left);

    if ai_on_left {
        // AI 왼쪽, 에디터 오른쪽
        if let Some(ref mut state) = app.ai_state {
            ai_screen::draw_with_focus(frame, state, panel_chunks[0], theme, false);
        }
        if let Some(ref mut state) = app.editor_state {
            file_editor::draw(frame, state, panel_chunks[1], theme);
        }
    } else {
        // 에디터 왼쪽, AI 오른쪽
        if let Some(ref mut state) = app.editor_state {
            file_editor::draw(frame, state, panel_chunks[0], theme);
        }
        if let Some(ref mut state) = app.ai_state {
            ai_screen::draw_with_focus(frame, state, panel_chunks[1], theme, false);
        }
    }

    // Status bar and Function bar
    draw_status_bar(frame, app, chunks[1], theme);
    draw_function_bar(frame, app, chunks[2], theme);
}

/// AI 모드에서 뷰어와 AI 화면을 나란히 표시
fn draw_viewer_with_ai(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    // Layout: Panels, Status Bar, Function Bar (same as draw_dual_panel)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // Panels
            Constraint::Length(1), // Status bar
            Constraint::Length(1), // Function bar
        ])
        .split(area);

    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[0]);

    let ai_on_left = app.ai_panel_side == Some(PanelSide::Left);

    if ai_on_left {
        // AI 왼쪽, 뷰어 오른쪽
        if let Some(ref mut state) = app.ai_state {
            ai_screen::draw_with_focus(frame, state, panel_chunks[0], theme, false);
        }
        if let Some(ref mut state) = app.viewer_state {
            file_viewer::draw(frame, state, panel_chunks[1], theme);
        }
    } else {
        // 뷰어 왼쪽, AI 오른쪽
        if let Some(ref mut state) = app.viewer_state {
            file_viewer::draw(frame, state, panel_chunks[0], theme);
        }
        if let Some(ref mut state) = app.ai_state {
            ai_screen::draw_with_focus(frame, state, panel_chunks[1], theme, false);
        }
    }

    // Status bar and Function bar
    draw_status_bar(frame, app, chunks[1], theme);
    draw_function_bar(frame, app, chunks[2], theme);
}
