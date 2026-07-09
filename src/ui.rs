//! Terminal rendering (ratatui) and the RAII terminal guard.
//!
//! `render` is a pure function of `&App` -> frame, so it can be
//! exercised with ratatui's `TestBackend`. `Tui` owns raw mode and the
//! alternate screen and restores them on drop or panic. The event loop
//! lives in `main`.

use crate::app::{App, ConfirmKind, Mode, SubField};
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
        let list = List::new(items).highlight_symbol("> ");
        let mut state = ListState::default();
        state.select(Some(app.selected));
        frame.render_stateful_widget(list, chunks[1], &mut state);
    }

    frame.render_widget(status(app), chunks[2]);
}

fn title(app: &App) -> Paragraph<'static> {
    let dry = if app.dry_run { "  [dry-run]" } else { "" };
    let text = format!(
        "git redate  -  {} commit(s)  [mode: {}]{}",
        app.commits.len(),
        app.edit_mode,
        dry
    );
    Paragraph::new(text).block(Block::default().borders(Borders::BOTTOM))
}

const HINTS: &str = "[up/down] row  [left/right] field  [+/-] adjust  [Space] expand  \
     [Tab] author/committer  [s] mode  [e] type  [c] copy-prev  [=] spread  [u] reset  \
     [?] help  [w] write  [q] quit";

fn status(app: &App) -> Paragraph<'static> {
    let line = match &app.mode {
        Mode::Editing { buffer } => {
            format!("type date (YYYY-MM-DD HH:MM): {buffer}_    [Enter] apply  [Esc] cancel")
        }
        Mode::Confirm { kind } => {
            let n = app.commits.iter().filter(|c| c.changed()).count();
            match kind {
                ConfirmKind::Write => {
                    format!("rewrite {n} commit(s)?   [y] yes    [n/Esc] no")
                }
                ConfirmKind::Quit => {
                    format!("discard {n} change(s) and quit?   [y] yes    [n/Esc] no")
                }
            }
        }
        Mode::Navigate => match &app.message {
            Some(msg) => msg.clone(),
            None => HINTS.to_string(),
        },
    };
    Paragraph::new(line)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::TOP))
}

fn help_widget() -> Paragraph<'static> {
    let lines = vec![
        Line::from("git-redate keys"),
        Line::from(""),
        Line::from("  up/down, k/j        select commit"),
        Line::from("  left/right, h/l     move date field"),
        Line::from("  +/-, shift+up/dn    adjust the field (calendar carry)"),
        Line::from("  ctrl-a / ctrl-x     adjust the field (vim-style)"),
        Line::from("  Space               expand author/committer (and offset)"),
        Line::from("  Tab / shift-Tab     switch author <-> committer (expanded)"),
        Line::from("  s                   toggle single <-> shift (cascade)"),
        Line::from("  e / Enter           type an absolute date"),
        Line::from("  c                   copy the previous (older) commit's time"),
        Line::from("  =                   spread commits evenly in time"),
        Line::from("  u                   reset the selected commit"),
        Line::from(""),
        Line::from("  w / W               write (confirm / force)"),
        Line::from("  q / Q, Esc          quit (confirm / force)   ctrl-c  abort"),
        Line::from("  ? / F1              toggle this help"),
        Line::from(""),
        Line::from("  shift mode: editing a commit moves it and every newer"),
        Line::from("  commit by the same delta, keeping the gaps."),
    ];
    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" help "))
}

/// The rendered line(s) for commit `i`.
fn commit_lines(app: &App, i: usize) -> Vec<Line<'static>> {
    let ec = &app.commits[i];
    let selected = i == app.selected && !app.is_editing();
    let changed = ec.changed();
    let marker = if changed { "*" } else { " " };

    if !ec.expanded {
        let focus = if selected { Some(app.component) } else { None };
        let date = datetime::format(ec.author);
        let mut spans = vec![Span::raw(format!("{marker}{}  ", ec.original.short_id))];
        spans.extend(date_spans(&date, None, focus, changed));
        spans.push(Span::raw(format!("  {}", ec.original.summary)));
        vec![Line::from(spans)]
    } else {
        let header = Line::from(format!(
            "{marker}{}  {}",
            ec.original.short_id, ec.original.summary
        ));
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
    let mut spans = vec![Span::raw(format!("    {label}  "))];
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
        Style::default().fg(Color::Green)
    } else {
        Style::default()
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
}
