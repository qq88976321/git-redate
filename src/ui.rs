//! Terminal rendering (ratatui) and the RAII terminal guard.
//!
//! `render` is a pure function of `&App` -> frame, so it can be
//! exercised with ratatui's `TestBackend`. `Tui` owns raw mode and the
//! alternate screen and restores them on drop or panic. The event loop
//! lives in `main`.

use crate::app::{App, ConfirmKind, Mode, SubField};
use crate::cli::EditMode;
use crate::datetime::{self, Component, Stamp};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};

// Catppuccin Mocha palette (fixed RGB, dark flavor; https://catppuccin.com).
// Foreground-only: the terminal's own background is kept, so nothing is
// repainted and the scheme blends into the surrounding shell.
const MAUVE: Color = Color::Rgb(0xcb, 0xa6, 0xf7);
const GREEN: Color = Color::Rgb(0xa6, 0xe3, 0xa1);
const PEACH: Color = Color::Rgb(0xfa, 0xb3, 0x87);
const YELLOW: Color = Color::Rgb(0xf9, 0xe2, 0xaf);
const RED: Color = Color::Rgb(0xf3, 0x8b, 0xa8);
const BLUE: Color = Color::Rgb(0x89, 0xb4, 0xfa);
const SKY: Color = Color::Rgb(0x89, 0xdc, 0xeb);
const OVERLAY1: Color = Color::Rgb(0x7f, 0x84, 0x9c);

// Semantic roles mapped onto the palette.
const ACCENT: Color = MAUVE; // brand title, cursor, help title
const INPUT: Color = BLUE; // edit prompt and help keys (interactive)
const TIME: Color = SKY; // unedited timestamps (distinct from summaries)
const CHANGED: Color = GREEN; // edited timestamps and the "*" marker
const DIM: Color = OVERLAY1; // hashes, labels, footer hints, borders
const CAUTION: Color = PEACH; // dry-run, shift mode, write confirmation
const INFO: Color = YELLOW; // transient status message
const DANGER: Color = RED; // discard-and-quit confirmation

/// Draw the whole editor.
pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(frame.area());

    frame.render_widget(title(app), chunks[0]);

    if app.show_help {
        frame.render_widget(help_widget(), chunks[1]);
    } else {
        let items: Vec<ListItem> = (0..app.commits.len())
            .map(|i| ListItem::new(commit_lines(app, i)))
            .collect();
        let list = List::new(items)
            .highlight_symbol(Line::from(Span::styled(
                "> ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD));
        let mut state = ListState::default();
        state.select(Some(app.selected));
        frame.render_stateful_widget(list, chunks[1], &mut state);
    }

    frame.render_widget(status(app), chunks[2]);
}

fn title(app: &App) -> Paragraph<'static> {
    let mode_style = match app.edit_mode {
        EditMode::Shift => Style::default().fg(CAUTION),
        EditMode::Single => Style::default().fg(DIM),
    };
    let mut spans = vec![
        Span::styled(
            "git redate",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  -  {} commit(s)  ", app.commits.len())),
        Span::styled(format!("[mode: {}]", app.edit_mode), mode_style),
    ];
    if app.dry_run {
        spans.push(Span::styled(
            "  [dry-run]",
            Style::default().fg(CAUTION).add_modifier(Modifier::BOLD),
        ));
    }
    Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(DIM)),
    )
}

const HINTS: &str = "[up/down] row  [left/right] field  [+/-] adjust  [Space] expand  \
     [Tab] author/committer  [s] mode  [e] type  [c] copy-prev  [=] spread  [u] reset  \
     [?] help  [w] write  [q] quit";

fn status(app: &App) -> Paragraph<'static> {
    let dim = Style::default().fg(DIM);
    let line = match &app.mode {
        Mode::Editing { buffer } => Line::from(vec![
            Span::styled("type date (YYYY-MM-DD HH:MM): ", Style::default().fg(INPUT)),
            Span::styled(
                format!("{buffer}_"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("    [Enter] apply  [Esc] cancel", dim),
        ]),
        Mode::Confirm { kind } => {
            let n = app.commits.iter().filter(|c| c.changed()).count();
            let (prompt, style) = match kind {
                ConfirmKind::Write => (
                    format!("rewrite {n} commit(s)?"),
                    Style::default().fg(CAUTION).add_modifier(Modifier::BOLD),
                ),
                ConfirmKind::Quit => (
                    format!("discard {n} change(s) and quit?"),
                    Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
                ),
                ConfirmKind::ResetAll => (
                    format!("discard {n} change(s) and reset all?"),
                    Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
                ),
            };
            Line::from(vec![
                Span::styled(prompt, style),
                Span::styled("   [y] yes    [n/Esc] no", dim),
            ])
        }
        Mode::Navigate => match &app.message {
            Some(msg) => Line::from(Span::styled(msg.clone(), Style::default().fg(INFO))),
            None => Line::from(Span::styled(HINTS, dim)),
        },
    };
    Paragraph::new(line).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(DIM)),
    )
}

fn help_widget() -> Paragraph<'static> {
    let head = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(DIM);
    // Key column styled INPUT, description left in the default fg. The
    // key text is padded to a fixed width so the two columns line up.
    let entry = |keys: &str, desc: &str| {
        Line::from(vec![
            Span::styled(format!("  {keys:<20}"), Style::default().fg(INPUT)),
            Span::raw(desc.to_string()),
        ])
    };
    let lines = vec![
        Line::from(Span::styled("git-redate keys", head)),
        Line::from(""),
        entry("up/down, k/j", "select commit"),
        entry("left/right, h/l", "move date field"),
        entry("+/-, shift+up/dn", "adjust the field (calendar carry)"),
        entry("ctrl-a / ctrl-x", "adjust the field (vim-style)"),
        entry("Space", "expand author/committer (and offset)"),
        entry("Tab / shift-Tab", "switch author <-> committer (expanded)"),
        entry("s", "toggle single <-> shift (cascade)"),
        entry("e / Enter", "type an absolute date"),
        entry("c", "copy the previous (older) commit's time"),
        entry("=", "spread commits evenly in time"),
        entry("u", "reset the selected commit"),
        entry("U", "reset all commits"),
        entry("ctrl-z / ctrl-r", "undo / redo the last edit"),
        Line::from(""),
        entry("w / W", "write (confirm / force)"),
        entry("q / Q, Esc", "quit (confirm / force)   ctrl-c  abort"),
        entry("? / F1", "toggle this help"),
        Line::from(""),
        Line::from(Span::styled(
            "  shift mode: editing a commit moves it and every newer",
            dim,
        )),
        Line::from(Span::styled(
            "  commit by the same delta, keeping the gaps.",
            dim,
        )),
    ];
    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(dim)
            .title(" help ")
            .title_style(head),
    )
}

/// The rendered line(s) for commit `i`.
fn commit_lines(app: &App, i: usize) -> Vec<Line<'static>> {
    let ec = &app.commits[i];
    let selected = i == app.selected && !app.is_editing();
    let changed = ec.changed();
    let marker = if changed {
        Span::styled("*", Style::default().fg(CHANGED))
    } else {
        Span::raw(" ")
    };
    let hash = Span::styled(
        format!("{}  ", ec.original.short_id),
        Style::default().fg(DIM),
    );

    if !ec.expanded {
        let focus = if selected { Some(app.component) } else { None };
        let date = datetime::format(ec.author);
        let mut spans = vec![marker, hash];
        spans.extend(date_spans(&date, None, focus, changed));
        spans.push(Span::raw(format!("  {}", ec.original.summary)));
        vec![Line::from(spans)]
    } else {
        let header = Line::from(vec![marker, hash, Span::raw(ec.original.summary.clone())]);
        let a_focus = focus_for(app, selected, SubField::Author);
        let c_focus = focus_for(app, selected, SubField::Committer);
        vec![
            header,
            sub_line("author", ec.author, a_focus, changed),
            sub_line("commit", ec.committer, c_focus, changed),
        ]
    }
}

fn focus_for(app: &App, selected: bool, which: SubField) -> Option<Component> {
    if selected && app.sub == which {
        Some(app.component)
    } else {
        None
    }
}

fn sub_line(label: &str, stamp: Stamp, focus: Option<Component>, changed: bool) -> Line<'static> {
    let date = datetime::format(stamp);
    let offset = datetime::format_offset(stamp);
    let mut spans = vec![Span::styled(
        format!("    {label}  "),
        Style::default().fg(DIM),
    )];
    spans.extend(date_spans(&date, Some(&offset), focus, changed));
    Line::from(spans)
}

/// Split a `YYYY-MM-DD HH:MM` string into spans, reverse-highlighting
/// the focused component (or the offset). ASCII, so byte slicing is
/// safe.
fn date_spans(
    date: &str,
    offset: Option<&str>,
    focus: Option<Component>,
    changed: bool,
) -> Vec<Span<'static>> {
    let base = if changed {
        Style::default().fg(CHANGED)
    } else {
        Style::default().fg(TIME)
    };
    let hl = base.add_modifier(Modifier::REVERSED);

    let mut spans = Vec::new();
    match focus.and_then(comp_range) {
        Some((s, e)) => {
            spans.push(Span::styled(date[..s].to_string(), base));
            spans.push(Span::styled(date[s..e].to_string(), hl));
            spans.push(Span::styled(date[e..].to_string(), base));
        }
        None => spans.push(Span::styled(date.to_string(), base)),
    }
    if let Some(off) = offset {
        let off_style = if focus == Some(Component::Offset) {
            hl
        } else {
            base
        };
        spans.push(Span::styled(format!(" {off}"), off_style));
    }
    spans
}

fn comp_range(c: Component) -> Option<(usize, usize)> {
    match c {
        Component::Year => Some((0, 4)),
        Component::Month => Some((5, 7)),
        Component::Day => Some((8, 10)),
        Component::Hour => Some((11, 13)),
        Component::Minute => Some((14, 16)),
        Component::Offset => None,
    }
}

/// RAII terminal state: raw mode + alternate screen, restored on drop
/// and via a panic hook so a panic never leaves the terminal wedged.
pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = Tui::restore_terminal();
            prev(info);
        }));
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Tui { terminal })
    }

    pub fn draw(&mut self, app: &App) -> io::Result<()> {
        self.terminal.draw(|f| render(f, app))?;
        Ok(())
    }

    fn restore_terminal() -> io::Result<()> {
        execute!(io::stdout(), LeaveAlternateScreen)?;
        disable_raw_mode()
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = Tui::restore_terminal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::cli::EditMode;
    use crate::datetime::parse_in_offset;
    use crate::model::{Commit, EditableCommit};
    use ratatui::backend::TestBackend;

    fn app() -> App {
        let commits = ["2024-01-01 01:00", "2024-01-02 02:30"]
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let s = parse_in_offset(w, 0).unwrap();
                EditableCommit::new(Commit {
                    id: format!("{i:040x}"),
                    short_id: format!("abc{i}"),
                    summary: format!("summary {i}"),
                    author: s,
                    committer: s,
                })
            })
            .collect();
        App::new(commits, EditMode::Single, false, false)
    }

    fn rendered(app: &App) -> String {
        let backend = TestBackend::new(90, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn renders_title_and_commits() {
        let content = rendered(&app());
        assert!(content.contains("git redate"));
        assert!(content.contains("summary 0"));
        assert!(content.contains("2024-01-01 01:00"));
        assert!(content.contains("mode: single"));
    }

    #[test]
    fn expanded_row_shows_author_and_committer() {
        let mut a = app();
        a.commits[0].expanded = true;
        let content = rendered(&a);
        assert!(content.contains("author"));
        assert!(content.contains("commit"));
        assert!(content.contains("+00:00"));
    }

    #[test]
    fn editing_shows_the_buffer_in_the_status_bar() {
        let mut a = app();
        a.mode = Mode::Editing {
            buffer: "2024-06-15 09:30".to_string(),
        };
        let content = rendered(&a);
        assert!(content.contains("2024-06-15 09:30"));
    }

    #[test]
    fn help_overlay_renders() {
        let mut a = app();
        a.show_help = true;
        let content = rendered(&a);
        assert!(content.contains("git-redate keys"));
    }

    #[test]
    fn write_confirm_prompt_shows_count() {
        let mut a = app();
        a.commits[0].author = parse_in_offset("2024-02-02 02:00", 0).unwrap();
        a.mode = Mode::Confirm {
            kind: ConfirmKind::Write,
        };
        let content = rendered(&a);
        assert!(content.contains("rewrite 1 commit"));
        assert!(content.contains("[y] yes"));
    }

    #[test]
    fn quit_confirm_prompt_reads_discard() {
        let mut a = app();
        a.commits[0].author = parse_in_offset("2024-02-02 02:00", 0).unwrap();
        a.mode = Mode::Confirm {
            kind: ConfirmKind::Quit,
        };
        let content = rendered(&a);
        assert!(content.contains("discard 1 change"));
    }

    #[test]
    fn reset_all_confirm_prompt_reads_reset_all() {
        let mut a = app();
        a.commits[0].author = parse_in_offset("2024-02-02 02:00", 0).unwrap();
        a.mode = Mode::Confirm {
            kind: ConfirmKind::ResetAll,
        };
        let content = rendered(&a);
        assert!(content.contains("reset all?"));
    }

    #[test]
    fn palette_is_applied_to_title_and_changed_rows() {
        use ratatui::style::Color;

        // The brand title "git redate" is drawn in the accent (mauve).
        let a = app();
        let backend = TestBackend::new(90, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &a)).unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf.cell((0, 0)).unwrap().fg, Color::Rgb(0xcb, 0xa6, 0xf7));

        // An unedited timestamp is sky (distinct from the default summary).
        let sky = Color::Rgb(0x89, 0xdc, 0xeb);
        let has_sky = (0u16..90).any(|x| buf.cell((x, 2)).unwrap().fg == sky);
        assert!(
            has_sky,
            "unedited commit row should contain sky timestamp cells"
        );

        // Editing a commit turns its timestamp (and marker) green.
        let mut b = app();
        b.commits[0].author = parse_in_offset("2024-02-02 02:00", 0).unwrap();
        let backend = TestBackend::new(90, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &b)).unwrap();
        let buf = terminal.backend().buffer();
        let green = Color::Rgb(0xa6, 0xe3, 0xa1);
        let has_green = (0u16..90).any(|x| buf.cell((x, 2)).unwrap().fg == green);
        assert!(has_green, "changed commit row should contain green cells");
    }
}
