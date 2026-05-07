//! Phase 10 — `/activity` scrollable modal.
//!
//! Displays the process-wide `EventLog` ring as a scrollable list. Users open
//! it by typing `/activity`. Keys:
//!   j / Down    — scroll down one entry
//!   k / Up      — scroll up one entry
//!   PgDn / PgUp — page scroll
//!   g / G       — jump to first / last entry
//!   f           — cycle source filter (all → main → cron → proactive → agent → bgloop → slash → system → all)
//!   d           — toggle details expansion for the selected row
//!   Esc / q     — close
//!
//! State lives on `App.activity_overlay`. Render path: `render_activity_overlay`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use claurst_core::event_log::{Event, EventKind, EventLog, ToolStatus};
use claurst_core::permissions::TaskSource;

use crate::overlays::centered_rect;

/// Source filter for the modal. Cycled with `f`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SourceFilter {
    #[default]
    All,
    Main,
    Cron,
    Proactive,
    Agent,
    BgLoop,
    Slash,
    System,
}

impl SourceFilter {
    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Main => "main",
            Self::Cron => "cron",
            Self::Proactive => "proactive",
            Self::Agent => "agent",
            Self::BgLoop => "bgloop",
            Self::Slash => "slash",
            Self::System => "system",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::All => Self::Main,
            Self::Main => Self::Cron,
            Self::Cron => Self::Proactive,
            Self::Proactive => Self::Agent,
            Self::Agent => Self::BgLoop,
            Self::BgLoop => Self::Slash,
            Self::Slash => Self::System,
            Self::System => Self::All,
        }
    }

    pub fn matches(&self, source: &TaskSource) -> bool {
        match (self, source) {
            (Self::All, _) => true,
            (Self::Main, TaskSource::MainSession) => true,
            (Self::Cron, TaskSource::Cron(_)) => true,
            (Self::Proactive, TaskSource::Proactive) => true,
            (Self::Agent, TaskSource::Agent(_)) => true,
            (Self::BgLoop, TaskSource::BgLoop(_)) => true,
            (Self::Slash, TaskSource::SlashCommand(_)) => true,
            (Self::System, TaskSource::System) => true,
            _ => false,
        }
    }
}

/// Modal state. Cheap to clone; default-constructable.
#[derive(Default)]
pub struct ActivityOverlay {
    pub visible: bool,
    pub filter: SourceFilter,
    /// Index into the filtered event slice currently selected (top of the
    /// rendered window when the list is shorter than the viewport).
    pub selected_idx: usize,
    /// First visible row index — driven by scroll keys.
    pub scroll_offset: usize,
    /// When `true`, the selected row's `details` field is expanded inline.
    pub expanded: bool,
}

impl ActivityOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.selected_idx = 0;
        self.scroll_offset = 0;
        self.expanded = false;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn toggle(&mut self) {
        if self.visible {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn cycle_filter(&mut self) {
        self.filter = self.filter.next();
        // Reset position when the filtered set changes shape.
        self.selected_idx = 0;
        self.scroll_offset = 0;
        self.expanded = false;
    }

    pub fn toggle_expand(&mut self) {
        self.expanded = !self.expanded;
    }

    pub fn move_down(&mut self, total: usize, viewport: usize) {
        if total == 0 {
            return;
        }
        if self.selected_idx + 1 < total {
            self.selected_idx += 1;
        }
        // Keep selected within scroll window.
        if self.selected_idx >= self.scroll_offset + viewport {
            self.scroll_offset = self.selected_idx + 1 - viewport;
        }
    }

    pub fn move_up(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
        if self.selected_idx < self.scroll_offset {
            self.scroll_offset = self.selected_idx;
        }
    }

    pub fn page_down(&mut self, total: usize, viewport: usize) {
        if total == 0 {
            return;
        }
        let step = viewport.max(1);
        self.selected_idx = (self.selected_idx + step).min(total - 1);
        if self.selected_idx >= self.scroll_offset + viewport {
            self.scroll_offset = self.selected_idx + 1 - viewport;
        }
    }

    pub fn page_up(&mut self, viewport: usize) {
        let step = viewport.max(1);
        self.selected_idx = self.selected_idx.saturating_sub(step);
        if self.selected_idx < self.scroll_offset {
            self.scroll_offset = self.selected_idx;
        }
    }

    pub fn jump_first(&mut self) {
        self.selected_idx = 0;
        self.scroll_offset = 0;
    }

    pub fn jump_last(&mut self, total: usize, viewport: usize) {
        if total == 0 {
            return;
        }
        self.selected_idx = total - 1;
        self.scroll_offset = total.saturating_sub(viewport);
    }
}

/// Build a label for an event source. Mirrors the format used by the text
/// `/activity` command.
fn source_label(source: &TaskSource) -> String {
    match source {
        TaskSource::MainSession => "main".to_string(),
        TaskSource::SlashCommand(n) => format!("/{}", n),
        TaskSource::Cron(id) => format!("cron:{}", id),
        TaskSource::Proactive => "proactive".to_string(),
        TaskSource::Agent(n) => format!("agent:{}", n),
        TaskSource::BgLoop(n) => format!("bg:{}", n),
        TaskSource::System => "system".to_string(),
    }
}

fn icon_for(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::TurnStart => "▶",
        EventKind::TurnEnd => "◼",
        EventKind::ToolCall { status, .. } => match status {
            ToolStatus::Started => "⚙",
            ToolStatus::Succeeded => "✓",
            ToolStatus::Failed => "✗",
            ToolStatus::Denied => "⛔",
        },
        EventKind::BackgroundStart => "→",
        EventKind::BackgroundFinish { is_error } => {
            if *is_error {
                "✗"
            } else {
                "✓"
            }
        }
        EventKind::PermissionRequested => "?",
        EventKind::PermissionDecided(_) => "!",
        EventKind::CronFired { .. } => "⏰",
        EventKind::AgentSpawned { .. } => "+",
        EventKind::ConfigChanged { .. } => "∆",
        EventKind::TaskPanicked { .. } => "☠",
        EventKind::SnapshotPartialLoad { .. } => "⚠",
        EventKind::Error(_) => "✗",
        EventKind::Info(_) => "ℹ",
    }
}

/// Apply the overlay's current filter to a snapshot.
pub fn filtered_events(overlay: &ActivityOverlay, log: &EventLog) -> Vec<Event> {
    let snap = log.snapshot();
    snap.into_iter()
        .filter(|e| overlay.filter.matches(&e.source))
        .collect()
}

/// Render the modal. No-op when `overlay.visible` is false.
pub fn render_activity_overlay(
    frame: &mut Frame,
    overlay: &ActivityOverlay,
    events: &[Event],
    area: Rect,
) {
    if !overlay.visible {
        return;
    }

    use crate::overlays::{render_dark_overlay, render_dialog_bg, CLAURST_PANEL_BG};

    let dialog = centered_rect(95, 80, area);
    render_dark_overlay(frame, area);
    render_dialog_bg(frame, dialog);
    frame.render_widget(Clear, dialog);

    let pink = Color::Rgb(233, 30, 99);
    let dim = Color::Rgb(110, 110, 124);
    let title_line = Line::from(vec![
        Span::styled(
            " /activity ",
            Style::default()
                .fg(Color::Black)
                .bg(pink)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("filter: {}", overlay.filter.label()),
            Style::default().fg(dim),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} event(s)", events.len()),
            Style::default().fg(dim),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pink))
        .title(title_line)
        .style(Style::default().bg(CLAURST_PANEL_BG));
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    // Reserve last row for footer; use everything above for the list.
    let footer_h: u16 = 1;
    let list_h = inner.height.saturating_sub(footer_h);
    let viewport = list_h as usize;

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(viewport.max(1));
    if events.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No events match the current filter.".to_string(),
            Style::default().fg(dim),
        )));
    } else {
        let start = overlay.scroll_offset.min(events.len());
        let end = (start + viewport).min(events.len());
        for (idx, e) in events[start..end].iter().enumerate() {
            let absolute = start + idx;
            let selected = absolute == overlay.selected_idx;
            let icon = icon_for(&e.kind);
            let src = source_label(&e.source);
            let row_style = if selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(60, 30, 50))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let prefix = if selected { "▸" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", prefix), row_style),
                Span::styled(
                    e.at.format("%H:%M:%S").to_string(),
                    Style::default().fg(dim),
                ),
                Span::raw(" "),
                Span::styled(icon.to_string(), Style::default().fg(pink)),
                Span::raw(" "),
                Span::styled(
                    format!("{:<14}", src.chars().take(14).collect::<String>()),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(" "),
                Span::styled(e.summary.clone(), row_style),
            ]));
            if selected && overlay.expanded {
                if let Some(details) = e.details.as_ref() {
                    for detail_line in details.lines() {
                        lines.push(Line::from(vec![
                            Span::raw("       "),
                            Span::styled(
                                detail_line.to_string(),
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::ITALIC),
                            ),
                        ]));
                    }
                } else {
                    lines.push(Line::from(Span::styled(
                        "       (no details)".to_string(),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
        }
    }

    let list_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: list_h,
    };
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        list_area,
    );

    // Footer hint row.
    let footer_area = Rect {
        x: inner.x,
        y: inner.y + list_h,
        width: inner.width,
        height: footer_h,
    };
    let hints = Line::from(vec![
        Span::styled("j/k", Style::default().fg(pink)),
        Span::styled(" scroll  ", Style::default().fg(dim)),
        Span::styled("PgUp/PgDn", Style::default().fg(pink)),
        Span::styled(" page  ", Style::default().fg(dim)),
        Span::styled("g/G", Style::default().fg(pink)),
        Span::styled(" jump  ", Style::default().fg(dim)),
        Span::styled("f", Style::default().fg(pink)),
        Span::styled(" filter  ", Style::default().fg(dim)),
        Span::styled("d", Style::default().fg(pink)),
        Span::styled(" details  ", Style::default().fg(dim)),
        Span::styled("Esc/q", Style::default().fg(pink)),
        Span::styled(" close", Style::default().fg(dim)),
    ]);
    frame.render_widget(Paragraph::new(hints), footer_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_filter_reaches_all_kinds() {
        let mut f = SourceFilter::All;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..16 {
            seen.insert(f);
            f = f.next();
        }
        // 8 variants → cycle returns to All within 8 steps.
        assert_eq!(seen.len(), 8);
    }

    #[test]
    fn filter_matches_correct_sources() {
        assert!(SourceFilter::Cron.matches(&TaskSource::Cron("x".into())));
        assert!(!SourceFilter::Cron.matches(&TaskSource::MainSession));
        assert!(SourceFilter::All.matches(&TaskSource::Proactive));
    }

    #[test]
    fn move_keys_track_scroll_offset() {
        let mut o = ActivityOverlay::new();
        o.open();
        o.move_down(10, 4);
        assert_eq!(o.selected_idx, 1);
        for _ in 0..6 {
            o.move_down(10, 4);
        }
        assert_eq!(o.selected_idx, 7);
        // Scroll offset should follow so selected stays in view.
        assert!(o.scroll_offset + 4 > o.selected_idx);
        o.move_up();
        assert_eq!(o.selected_idx, 6);
    }

    #[test]
    fn page_keys_advance_in_chunks() {
        let mut o = ActivityOverlay::new();
        o.open();
        o.page_down(20, 5);
        assert_eq!(o.selected_idx, 5);
        o.page_up(5);
        assert_eq!(o.selected_idx, 0);
    }

    #[test]
    fn jump_keys_go_to_ends() {
        let mut o = ActivityOverlay::new();
        o.open();
        o.jump_last(10, 4);
        assert_eq!(o.selected_idx, 9);
        assert_eq!(o.scroll_offset, 6);
        o.jump_first();
        assert_eq!(o.selected_idx, 0);
        assert_eq!(o.scroll_offset, 0);
    }

    #[test]
    fn toggle_visibility() {
        let mut o = ActivityOverlay::new();
        assert!(!o.visible);
        o.toggle();
        assert!(o.visible);
        o.toggle();
        assert!(!o.visible);
    }
}
