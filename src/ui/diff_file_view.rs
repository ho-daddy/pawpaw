use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use unicode_width::UnicodeWidthChar;

use super::app::App;
use super::theme::Theme;

// ═══════════════════════════════════════════════════════════════════════════════
// Data structures
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineStatus {
    Same,
    Modified,
    LeftOnly,
    RightOnly,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub left_line_no: Option<usize>,
    pub left_content: Option<String>,
    pub right_line_no: Option<usize>,
    pub right_content: Option<String>,
    pub line_status: DiffLineStatus,
}

pub struct DiffFileViewState {
    pub left_path: PathBuf,
    pub right_path: PathBuf,
    pub diff_lines: Vec<DiffLine>,
    pub scroll: usize,
    pub visible_height: usize,
    pub left_total_lines: usize,
    pub right_total_lines: usize,
    pub change_positions: Vec<usize>,
    pub current_change: usize,
    pub file_name: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Binary detection
// ═══════════════════════════════════════════════════════════════════════════════

/// Check if raw bytes represent a binary file by looking for null bytes in the first 8KB.
fn is_binary(data: &[u8]) -> bool {
    let check_len = data.len().min(8192);
    data[..check_len].contains(&0)
}

// ═══════════════════════════════════════════════════════════════════════════════
// LCS diff algorithm
// ═══════════════════════════════════════════════════════════════════════════════

/// Compute LCS (Longest Common Subsequence) of two line sequences.
/// Returns a list of matched pairs (left_index, right_index).
///
/// For files up to ~10000 lines each, uses standard O(n*m) DP.
/// For larger files, falls back to a simpler sequential comparison.
fn compute_lcs(left: &[String], right: &[String]) -> Vec<(usize, usize)> {
    let n = left.len();
    let m = right.len();

    // Size limit: if combined lines exceed 20000, use simple sequential matching
    if n + m > 20000 {
        return compute_lcs_simple(left, right);
    }

    // Standard O(n*m) DP approach
    // dp[i][j] = length of LCS of left[0..i] and right[0..j]
    let mut dp = vec![vec![0u32; m + 1]; n + 1];

    for i in 1..=n {
        for j in 1..=m {
            if left[i - 1] == right[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    // Backtrack to find actual LCS pairs
    let mut result = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 && j > 0 {
        if left[i - 1] == right[j - 1] {
            result.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }

    result.reverse();
    result
}

/// Simple sequential matching for large files.
/// Uses a greedy approach: for each left line, find the nearest unmatched right line.
fn compute_lcs_simple(left: &[String], right: &[String]) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let mut right_pos = 0;

    for (li, left_line) in left.iter().enumerate() {
        for ri in right_pos..right.len() {
            if left_line == &right[ri] {
                result.push((li, ri));
                right_pos = ri + 1;
                break;
            }
        }
    }

    result
}

/// Build diff lines from two line sequences using LCS matching.
fn build_diff_lines(
    left_lines: &[String],
    right_lines: &[String],
    lcs: &[(usize, usize)],
) -> (Vec<DiffLine>, Vec<usize>) {
    let mut diff_lines = Vec::new();
    let mut change_positions = Vec::new();

    let mut li = 0usize; // current position in left
    let mut ri = 0usize; // current position in right

    for &(lcs_li, lcs_ri) in lcs {
        // Process gap before this LCS match
        // Lines in left[li..lcs_li] and right[ri..lcs_ri] are not in LCS
        let left_gap = &left_lines[li..lcs_li];
        let right_gap = &right_lines[ri..lcs_ri];

        emit_gap(
            left_gap,
            right_gap,
            &mut li,
            &mut ri,
            &mut diff_lines,
            &mut change_positions,
        );

        // Emit the matching line
        diff_lines.push(DiffLine {
            left_line_no: Some(li + 1),
            left_content: Some(left_lines[lcs_li].clone()),
            right_line_no: Some(ri + 1),
            right_content: Some(right_lines[lcs_ri].clone()),
            line_status: DiffLineStatus::Same,
        });
        li = lcs_li + 1;
        ri = lcs_ri + 1;
    }

    // Process remaining lines after last LCS match
    let left_gap = &left_lines[li..];
    let right_gap = &right_lines[ri..];
    emit_gap(
        left_gap,
        right_gap,
        &mut li,
        &mut ri,
        &mut diff_lines,
        &mut change_positions,
    );

    (diff_lines, change_positions)
}

/// Emit diff lines for a gap between LCS matches.
/// Pairs up lines as Modified where both sides have content,
/// then emits remaining as LeftOnly or RightOnly.
fn emit_gap(
    left_gap: &[String],
    right_gap: &[String],
    li: &mut usize,
    ri: &mut usize,
    diff_lines: &mut Vec<DiffLine>,
    change_positions: &mut Vec<usize>,
) {
    let paired = left_gap.len().min(right_gap.len());

    for idx in 0..paired {
        let pos = diff_lines.len();
        if change_positions.last() != Some(&pos) {
            // Mark first line of a contiguous change block
            if idx == 0 {
                change_positions.push(pos);
            }
        }
        diff_lines.push(DiffLine {
            left_line_no: Some(*li + 1),
            left_content: Some(left_gap[idx].clone()),
            right_line_no: Some(*ri + 1),
            right_content: Some(right_gap[idx].clone()),
            line_status: DiffLineStatus::Modified,
        });
        *li += 1;
        *ri += 1;
    }

    // Remaining left-only lines
    for idx in paired..left_gap.len() {
        let pos = diff_lines.len();
        if paired == 0 && idx == 0 {
            change_positions.push(pos);
        } else if idx == paired && paired > 0 {
            // Already recorded by the modified block above
        } else if idx == paired {
            change_positions.push(pos);
        }
        diff_lines.push(DiffLine {
            left_line_no: Some(*li + 1),
            left_content: Some(left_gap[idx].clone()),
            right_line_no: None,
            right_content: None,
            line_status: DiffLineStatus::LeftOnly,
        });
        *li += 1;
    }

    // Remaining right-only lines
    for idx in paired..right_gap.len() {
        let pos = diff_lines.len();
        if paired == 0 && left_gap.is_empty() && idx == 0 {
            change_positions.push(pos);
        } else if idx == paired && paired > 0 {
            // Already recorded
        } else if idx == paired && left_gap.is_empty() {
            change_positions.push(pos);
        }
        diff_lines.push(DiffLine {
            left_line_no: None,
            left_content: None,
            right_line_no: Some(*ri + 1),
            right_content: Some(right_gap[idx].clone()),
            line_status: DiffLineStatus::RightOnly,
        });
        *ri += 1;
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// DiffFileViewState implementation
// ═══════════════════════════════════════════════════════════════════════════════

impl DiffFileViewState {
    pub fn new(left_path: PathBuf, right_path: PathBuf, file_name: String) -> Self {
        let left_data = fs::read(&left_path).ok();
        let right_data = fs::read(&right_path).ok();

        // Check for binary files
        let left_is_binary = left_data.as_ref().map_or(false, |d| is_binary(d));
        let right_is_binary = right_data.as_ref().map_or(false, |d| is_binary(d));

        if left_is_binary || right_is_binary {
            // Binary file: show a single informational line
            let diff_lines = vec![DiffLine {
                left_line_no: None,
                left_content: Some("Binary file".to_string()),
                right_line_no: None,
                right_content: Some("Binary file".to_string()),
                line_status: DiffLineStatus::Same,
            }];
            return Self {
                left_path,
                right_path,
                diff_lines,
                scroll: 0,
                visible_height: 0,
                left_total_lines: 0,
                right_total_lines: 0,
                change_positions: Vec::new(),
                current_change: 0,
                file_name,
            };
        }

        // Read as text, handle missing files gracefully
        let left_text = left_data
            .map(|d| String::from_utf8_lossy(&d).into_owned())
            .unwrap_or_default();
        let right_text = right_data
            .map(|d| String::from_utf8_lossy(&d).into_owned())
            .unwrap_or_default();

        let left_lines: Vec<String> = if left_text.is_empty() {
            Vec::new()
        } else {
            left_text.lines().map(|l| l.to_string()).collect()
        };
        let right_lines: Vec<String> = if right_text.is_empty() {
            Vec::new()
        } else {
            right_text.lines().map(|l| l.to_string()).collect()
        };

        let left_total_lines = left_lines.len();
        let right_total_lines = right_lines.len();

        // Handle case where one file doesn't exist (all LeftOnly or RightOnly)
        let (diff_lines, change_positions) = if left_lines.is_empty() && !right_lines.is_empty() {
            let mut diffs = Vec::new();
            let mut changes = Vec::new();
            if !right_lines.is_empty() {
                changes.push(0);
            }
            for (idx, line) in right_lines.iter().enumerate() {
                diffs.push(DiffLine {
                    left_line_no: None,
                    left_content: None,
                    right_line_no: Some(idx + 1),
                    right_content: Some(line.clone()),
                    line_status: DiffLineStatus::RightOnly,
                });
            }
            (diffs, changes)
        } else if !left_lines.is_empty() && right_lines.is_empty() {
            let mut diffs = Vec::new();
            let mut changes = Vec::new();
            if !left_lines.is_empty() {
                changes.push(0);
            }
            for (idx, line) in left_lines.iter().enumerate() {
                diffs.push(DiffLine {
                    left_line_no: Some(idx + 1),
                    left_content: Some(line.clone()),
                    right_line_no: None,
                    right_content: None,
                    line_status: DiffLineStatus::LeftOnly,
                });
            }
            (diffs, changes)
        } else {
            // Both files have content: compute LCS-based diff
            let lcs = compute_lcs(&left_lines, &right_lines);
            build_diff_lines(&left_lines, &right_lines, &lcs)
        };

        Self {
            left_path,
            right_path,
            diff_lines,
            scroll: 0,
            visible_height: 0,
            left_total_lines,
            right_total_lines,
            change_positions,
            current_change: 0,
            file_name,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Drawing
// ═══════════════════════════════════════════════════════════════════════════════

pub fn draw(frame: &mut Frame, state: &mut DiffFileViewState, area: Rect, theme: &Theme) {
    if area.height < 4 {
        return;
    }

    // Layout: Header(1) + Content(fill) + StatusBar(1) + FunctionBar(1)
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // Header
            Constraint::Min(3),    // Content
            Constraint::Length(1), // StatusBar
            Constraint::Length(1), // FunctionBar
        ])
        .split(area);

    let header_area = layout[0];
    let content_area = layout[1];
    let status_area = layout[2];
    let function_area = layout[3];

    // Update visible height
    state.visible_height = content_area.height as usize;

    // ─── Header ─────────────────────────────────────────────────────────────
    let header_text = format!("[FILE DIFF] {}", state.file_name);
    let header_line = Line::from(Span::styled(
        header_text,
        Style::default()
            .fg(theme.diff_file_view.header_text)
            .bg(theme.diff_file_view.bg),
    ));
    let header_paragraph = Paragraph::new(header_line)
        .style(Style::default().bg(theme.diff_file_view.bg));
    frame.render_widget(header_paragraph, header_area);

    // ─── Content: split 50:50 horizontal ────────────────────────────────────
    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(content_area);

    let left_area = content_layout[0];
    let right_area = content_layout[1];
    // Left panel has Borders::RIGHT which consumes 1 column
    let left_inner_width = (left_area.width as usize).saturating_sub(1);

    let visible_lines = state.visible_height;
    let total_lines = state.diff_lines.len();

    // Clamp scroll
    if total_lines > visible_lines {
        if state.scroll > total_lines - visible_lines {
            state.scroll = total_lines - visible_lines;
        }
    } else {
        state.scroll = 0;
    }

    let end = (state.scroll + visible_lines).min(total_lines);
    let visible_slice = &state.diff_lines[state.scroll..end];

    // Line number width: marker(1) + digits + separator(│)
    let max_line = state.left_total_lines.max(state.right_total_lines).max(1);
    let digit_width = ((max_line as f64).log10().floor() as usize) + 1;
    let line_no_width = 1 + digit_width; // 1 for marker + digits

    // Build left and right lines
    let mut left_lines_display: Vec<Line> = Vec::with_capacity(visible_lines);
    let mut right_lines_display: Vec<Line> = Vec::with_capacity(visible_lines);

    let current_change_pos = if !state.change_positions.is_empty() {
        Some(state.change_positions[state.current_change])
    } else {
        None
    };

    for (i, diff_line) in visible_slice.iter().enumerate() {
        let absolute_idx = state.scroll + i;
        let is_current_change = current_change_pos == Some(absolute_idx);
        let (left_spans, right_spans) =
            render_diff_line(diff_line, line_no_width, left_inner_width, right_area.width as usize, theme, is_current_change);
        left_lines_display.push(Line::from(left_spans));
        right_lines_display.push(Line::from(right_spans));
    }

    // Fill remaining lines with empty bg
    for _ in visible_slice.len()..visible_lines {
        left_lines_display.push(Line::from(Span::styled(
            " ".repeat(left_inner_width),
            Style::default().bg(theme.diff_file_view.bg),
        )));
        right_lines_display.push(Line::from(Span::styled(
            " ".repeat(right_area.width as usize),
            Style::default().bg(theme.diff_file_view.bg),
        )));
    }

    // Render left panel
    let left_block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(theme.diff_file_view.border));
    let left_inner = left_block.inner(left_area);
    frame.render_widget(left_block, left_area);
    let left_paragraph = Paragraph::new(left_lines_display);
    frame.render_widget(left_paragraph, left_inner);

    // Render right panel
    let right_paragraph = Paragraph::new(right_lines_display);
    frame.render_widget(right_paragraph, right_area);

    // Scrollbar
    if total_lines > visible_lines {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state = ScrollbarState::new(total_lines.saturating_sub(visible_lines))
            .position(state.scroll);
        frame.render_stateful_widget(scrollbar, content_area, &mut scrollbar_state);
    }

    // ─── StatusBar ──────────────────────────────────────────────────────────
    let changes_count = state.change_positions.len();
    let current_display = if changes_count > 0 {
        state.current_change + 1
    } else {
        0
    };
    let status_text = format!(
        " Lines: {}/{} Changes: {} [{}/{}]",
        state.left_total_lines,
        state.right_total_lines,
        changes_count,
        current_display,
        changes_count,
    );
    let status_line = Line::from(Span::styled(
        status_text,
        Style::default()
            .fg(theme.diff_file_view.status_bar_text)
            .bg(theme.diff_file_view.status_bar_bg),
    ));
    let status_paragraph = Paragraph::new(status_line)
        .style(Style::default().bg(theme.diff_file_view.status_bar_bg));
    frame.render_widget(status_paragraph, status_area);

    // ─── FunctionBar ────────────────────────────────────────────────────────
    let fn_spans = vec![
        Span::styled(
            " \u{2191}\u{2193}",
            Style::default()
                .fg(theme.diff_file_view.footer_key)
                .bg(theme.diff_file_view.bg),
        ),
        Span::styled(
            ":scroll ",
            Style::default()
                .fg(theme.diff_file_view.footer_text)
                .bg(theme.diff_file_view.bg),
        ),
        Span::styled(
            "PgUp/Dn",
            Style::default()
                .fg(theme.diff_file_view.footer_key)
                .bg(theme.diff_file_view.bg),
        ),
        Span::styled(
            ":page ",
            Style::default()
                .fg(theme.diff_file_view.footer_text)
                .bg(theme.diff_file_view.bg),
        ),
        Span::styled(
            "n/p",
            Style::default()
                .fg(theme.diff_file_view.footer_key)
                .bg(theme.diff_file_view.bg),
        ),
        Span::styled(
            ":next/prev change ",
            Style::default()
                .fg(theme.diff_file_view.footer_text)
                .bg(theme.diff_file_view.bg),
        ),
        Span::styled(
            "Esc",
            Style::default()
                .fg(theme.diff_file_view.footer_key)
                .bg(theme.diff_file_view.bg),
        ),
        Span::styled(
            ":back",
            Style::default()
                .fg(theme.diff_file_view.footer_text)
                .bg(theme.diff_file_view.bg),
        ),
    ];
    let fn_line = Line::from(fn_spans);
    let fn_paragraph = Paragraph::new(fn_line)
        .style(Style::default().bg(theme.diff_file_view.bg));
    frame.render_widget(fn_paragraph, function_area);
}

/// Render a single DiffLine into left and right Span vectors.
fn render_diff_line<'a>(
    diff_line: &DiffLine,
    line_no_width: usize,
    left_width: usize,
    right_width: usize,
    theme: &Theme,
    is_current_change: bool,
) -> (Vec<Span<'a>>, Vec<Span<'a>>) {
    let colors = &theme.diff_file_view;

    // Determine styles based on status
    let (left_style, right_style, left_empty, right_empty) = match diff_line.line_status {
        DiffLineStatus::Same => {
            let s = Style::default().fg(colors.same_text).bg(colors.bg);
            (s, s, false, false)
        }
        DiffLineStatus::Modified => {
            let s = Style::default().fg(colors.modified_text).bg(colors.modified_bg);
            (s, s, false, false)
        }
        DiffLineStatus::LeftOnly => {
            let ls = Style::default().fg(colors.left_only_text).bg(colors.left_only_bg);
            let rs = Style::default().bg(colors.empty_bg);
            (ls, rs, false, true)
        }
        DiffLineStatus::RightOnly => {
            let ls = Style::default().bg(colors.empty_bg);
            let rs = Style::default().fg(colors.right_only_text).bg(colors.right_only_bg);
            (ls, rs, true, false)
        }
    };

    let line_no_style = Style::default().fg(colors.line_number).bg(
        match diff_line.line_status {
            DiffLineStatus::Same => colors.bg,
            DiffLineStatus::Modified => colors.modified_bg,
            DiffLineStatus::LeftOnly => colors.left_only_bg,
            DiffLineStatus::RightOnly => colors.empty_bg,
        },
    );

    let line_no_style_right = Style::default().fg(colors.line_number).bg(
        match diff_line.line_status {
            DiffLineStatus::Same => colors.bg,
            DiffLineStatus::Modified => colors.modified_bg,
            DiffLineStatus::LeftOnly => colors.empty_bg,
            DiffLineStatus::RightOnly => colors.right_only_bg,
        },
    );

    // Current change marker
    let marker = if is_current_change { "\u{25B6}" } else { " " };
    let num_width = line_no_width.saturating_sub(1); // 1 char reserved for marker

    // Inline change style for character-level highlighting within Modified lines
    let inline_style = Style::default()
        .fg(colors.inline_change_text)
        .bg(colors.inline_change_bg);

    // Left side
    let left_spans = if left_empty {
        // Empty placeholder for RightOnly lines
        let no_str = format!("{}{:>width$}\u{2502}", marker, "", width = num_width);
        let content_width = left_width.saturating_sub(line_no_width + 1);
        vec![
            Span::styled(no_str, Style::default().fg(colors.line_number).bg(colors.empty_bg)),
            Span::styled(
                format!("{:<width$}", "", width = content_width),
                Style::default().bg(colors.empty_bg),
            ),
        ]
    } else {
        let no_str = match diff_line.left_line_no {
            Some(n) => format!("{}{:>width$}\u{2502}", marker, n, width = num_width),
            None => format!("{}{:>width$}\u{2502}", marker, "", width = num_width),
        };
        let content_width = left_width.saturating_sub(line_no_width + 1);
        let left_content = diff_line.left_content.as_deref().unwrap_or("");
        if diff_line.line_status == DiffLineStatus::Modified {
            let right_content = diff_line.right_content.as_deref().unwrap_or("");
            let mut spans = vec![Span::styled(no_str, line_no_style)];
            spans.extend(build_inline_spans(left_content, right_content, content_width, left_style, inline_style));
            spans
        } else {
            let display_content = truncate_or_pad(left_content, content_width);
            vec![
                Span::styled(no_str, line_no_style),
                Span::styled(display_content, left_style),
            ]
        }
    };

    // Right side
    let right_spans = if right_empty {
        // Empty placeholder for LeftOnly lines
        let no_str = format!("{}{:>width$}\u{2502}", marker, "", width = num_width);
        let content_width = right_width.saturating_sub(line_no_width + 1);
        vec![
            Span::styled(no_str, Style::default().fg(colors.line_number).bg(colors.empty_bg)),
            Span::styled(
                format!("{:<width$}", "", width = content_width),
                Style::default().bg(colors.empty_bg),
            ),
        ]
    } else {
        let no_str = match diff_line.right_line_no {
            Some(n) => format!("{}{:>width$}\u{2502}", marker, n, width = num_width),
            None => format!("{}{:>width$}\u{2502}", marker, "", width = num_width),
        };
        let content_width = right_width.saturating_sub(line_no_width + 1);
        let right_content = diff_line.right_content.as_deref().unwrap_or("");
        if diff_line.line_status == DiffLineStatus::Modified {
            let left_content = diff_line.left_content.as_deref().unwrap_or("");
            let mut spans = vec![Span::styled(no_str, line_no_style_right)];
            spans.extend(build_inline_spans(right_content, left_content, content_width, right_style, inline_style));
            spans
        } else {
            let display_content = truncate_or_pad(right_content, content_width);
            vec![
                Span::styled(no_str, line_no_style_right),
                Span::styled(display_content, right_style),
            ]
        }
    };

    (left_spans, right_spans)
}

/// Truncate string to fit display width, or pad with spaces to fill.
/// Handles CJK/fullwidth characters correctly (2 columns each).
fn truncate_or_pad(s: &str, width: usize) -> String {
    let mut result = String::with_capacity(width);
    let mut display_width = 0;

    for ch in s.chars() {
        if ch == '\t' {
            // Expand tab to spaces
            let spaces = 4 - (display_width % 4);
            for _ in 0..spaces {
                if display_width >= width {
                    break;
                }
                result.push(' ');
                display_width += 1;
            }
        } else {
            let ch_width = ch.width().unwrap_or(0);
            if display_width + ch_width > width {
                break;
            }
            result.push(ch);
            display_width += ch_width;
        }
    }

    // Pad remaining with spaces
    while display_width < width {
        result.push(' ');
        display_width += 1;
    }

    result
}

/// Expand a string into a flat character list with tabs expanded to spaces.
fn expand_chars(s: &str) -> Vec<char> {
    let mut result = Vec::new();
    let mut col = 0;
    for ch in s.chars() {
        if ch == '\t' {
            let spaces = 4 - (col % 4);
            for _ in 0..spaces {
                result.push(' ');
                col += 1;
            }
        } else {
            result.push(ch);
            col += ch.width().unwrap_or(0);
        }
    }
    result
}

/// Build inline diff spans for Modified lines, highlighting character-level differences.
/// `this_content` is the content to render, `other_content` is the opposite side for comparison.
fn build_inline_spans<'a>(
    this_content: &str,
    other_content: &str,
    width: usize,
    base_style: Style,
    inline_style: Style,
) -> Vec<Span<'a>> {
    let this_chars = expand_chars(this_content);
    let other_chars = expand_chars(other_content);

    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut buf = String::new();
    let mut buf_is_diff = false;
    let mut display_width = 0;
    let max_len = this_chars.len().max(other_chars.len());

    for i in 0..max_len {
        if display_width >= width {
            break;
        }

        let this_ch = this_chars.get(i).copied();
        let other_ch = other_chars.get(i).copied();

        let ch = this_ch.unwrap_or(' ');
        let ch_w = ch.width().unwrap_or(0);

        if display_width + ch_w > width {
            break;
        }

        let is_diff = this_ch != other_ch;

        if is_diff != buf_is_diff && !buf.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut buf),
                if buf_is_diff { inline_style } else { base_style },
            ));
        }
        buf_is_diff = is_diff;
        buf.push(ch);
        display_width += ch_w;
    }

    if !buf.is_empty() {
        spans.push(Span::styled(
            buf,
            if buf_is_diff { inline_style } else { base_style },
        ));
    }

    // Pad remaining with spaces
    if display_width < width {
        spans.push(Span::styled(
            " ".repeat(width - display_width),
            base_style,
        ));
    }

    spans
}

// ═══════════════════════════════════════════════════════════════════════════════
// Input handling
// ═══════════════════════════════════════════════════════════════════════════════

pub fn handle_input(app: &mut App, code: KeyCode, _modifiers: KeyModifiers) {
    let state = match app.diff_file_view_state.as_mut() {
        Some(s) => s,
        None => return,
    };

    let visible = state.visible_height;
    let total = state.diff_lines.len();
    let max_scroll = total.saturating_sub(visible);

    match code {
        KeyCode::Up => {
            state.scroll = state.scroll.saturating_sub(1);
        }
        KeyCode::Down => {
            if state.scroll < max_scroll {
                state.scroll += 1;
            }
        }
        KeyCode::PageUp => {
            state.scroll = state.scroll.saturating_sub(visible);
        }
        KeyCode::PageDown => {
            state.scroll = (state.scroll + visible).min(max_scroll);
        }
        KeyCode::Home => {
            state.scroll = 0;
        }
        KeyCode::End => {
            state.scroll = max_scroll;
        }
        KeyCode::Char('n') => {
            // Jump to next change position (or scroll to current if not visible)
            if !state.change_positions.is_empty() {
                if state.current_change + 1 < state.change_positions.len() {
                    state.current_change += 1;
                }
                let target = state.change_positions[state.current_change];
                state.scroll = target.saturating_sub(visible / 4).min(max_scroll);
            }
        }
        KeyCode::Char('N') | KeyCode::Char('p') | KeyCode::Char('P') => {
            // Jump to previous change position (or scroll to current if not visible)
            if !state.change_positions.is_empty() {
                if state.current_change > 0 {
                    state.current_change -= 1;
                }
                let target = state.change_positions[state.current_change];
                state.scroll = target.saturating_sub(visible / 4).min(max_scroll);
            }
        }
        KeyCode::Esc => {
            // Go back to DiffScreen
            app.current_screen = super::app::Screen::DiffScreen;
            app.diff_file_view_state = None;
        }
        _ => {}
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_lcs_identical() {
        let left = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let right = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let lcs = compute_lcs(&left, &right);
        assert_eq!(lcs, vec![(0, 0), (1, 1), (2, 2)]);
    }

    #[test]
    fn test_compute_lcs_different() {
        let left = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let right = vec!["x".to_string(), "y".to_string(), "z".to_string()];
        let lcs = compute_lcs(&left, &right);
        assert!(lcs.is_empty());
    }

    #[test]
    fn test_compute_lcs_partial() {
        let left = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let right = vec!["a".to_string(), "x".to_string(), "c".to_string()];
        let lcs = compute_lcs(&left, &right);
        assert_eq!(lcs, vec![(0, 0), (2, 2)]);
    }

    #[test]
    fn test_build_diff_lines_same() {
        let left = vec!["line1".to_string(), "line2".to_string()];
        let right = vec!["line1".to_string(), "line2".to_string()];
        let lcs = compute_lcs(&left, &right);
        let (diff_lines, changes) = build_diff_lines(&left, &right, &lcs);
        assert_eq!(diff_lines.len(), 2);
        assert_eq!(diff_lines[0].line_status, DiffLineStatus::Same);
        assert_eq!(diff_lines[1].line_status, DiffLineStatus::Same);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_build_diff_lines_modified() {
        let left = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let right = vec!["a".to_string(), "x".to_string(), "c".to_string()];
        let lcs = compute_lcs(&left, &right);
        let (diff_lines, changes) = build_diff_lines(&left, &right, &lcs);
        assert_eq!(diff_lines.len(), 3);
        assert_eq!(diff_lines[0].line_status, DiffLineStatus::Same);
        assert_eq!(diff_lines[1].line_status, DiffLineStatus::Modified);
        assert_eq!(diff_lines[2].line_status, DiffLineStatus::Same);
        assert!(!changes.is_empty());
    }

    #[test]
    fn test_truncate_or_pad() {
        assert_eq!(truncate_or_pad("hello", 10), "hello     ");
        assert_eq!(truncate_or_pad("hello world!", 5), "hello");
        assert_eq!(truncate_or_pad("", 3), "   ");
    }

    #[test]
    fn test_is_binary() {
        assert!(is_binary(&[0x00, 0x01, 0x02]));
        assert!(!is_binary(b"hello world"));
        assert!(!is_binary(&[]));
    }

    #[test]
    fn test_compute_lcs_empty() {
        let left: Vec<String> = Vec::new();
        let right: Vec<String> = Vec::new();
        let lcs = compute_lcs(&left, &right);
        assert!(lcs.is_empty());
    }
}
