//! `git-redate` entry point: parse args, load the range, run the
//! interactive editor (or a dry-run preview), and rewrite history.

use anyhow::{Context, Result};
use clap::Parser;
use git_redate::app::App;
use git_redate::cli::{self, Cli};
use git_redate::datetime;
use git_redate::error::RedateError;
use git_redate::input;
use git_redate::model::EditableCommit;
use git_redate::rewrite::{RewriteReport, TagSig};
use git_redate::{config, repo, rewrite, ui};
use std::io::IsTerminal;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("git-redate: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let range = cli::normalize(&cli).context("resolving the requested range")?;

    let repository = repo::open().context("opening the repository")?;
    let loaded = repo::load(&repository, &range).context("selecting commits to edit")?;

    let git_mode = repo::config_edit_mode(&repository);
    let effective = config::resolve(cli.mode, git_mode.as_deref());
    if let Some(warning) = &effective.warning {
        eprintln!("git-redate: {warning}");
    }
    if loaded.dirty {
        eprintln!(
            "git-redate: note: the working tree has uncommitted changes; \
             they are preserved (only commit dates change)"
        );
    }

    // Tags pointing into the range move with their commits unless
    // --no-retag asks to leave them behind.
    let tag_scan = if cli.no_retag {
        repo::TagScan::default()
    } else {
        repo::tags_in_range(&repository, &loaded.commits).context("scanning tags")?
    };
    for warning in &tag_scan.skipped {
        eprintln!("git-redate: {warning}");
    }
    if !tag_scan.tags.is_empty() {
        eprintln!(
            "git-redate: note: {} tag(s) point at commits in this range; \
             they move with the commits that get rewritten \
             (--no-retag to leave them)",
            tag_scan.tags.len()
        );
    }

    let editable: Vec<EditableCommit> = loaded
        .commits
        .into_iter()
        .map(EditableCommit::new)
        .collect();
    let mut app = App::new(
        editable,
        effective.mode,
        cli.dry_run,
        cli.separate,
        tag_scan.tags.iter().map(|t| t.commit_index).collect(),
    );

    if cli.dry_run {
        // Edit interactively when possible, then print the plan without
        // writing anything. Headless, this just previews the range.
        if std::io::stdout().is_terminal() {
            run_tui(&mut app).context("running the editor")?;
        }
        print_plan(&app.commits, &tag_scan.tags);
        return Ok(ExitCode::SUCCESS);
    }

    if !std::io::stdout().is_terminal() {
        return Err(RedateError::NotATty.into());
    }

    run_tui(&mut app).context("running the editor")?;
    if !app.write_requested {
        println!("git-redate: cancelled; nothing written");
        return Ok(ExitCode::SUCCESS);
    }

    // Re-sign originally-signed commits with the repo's signing config,
    // unless --no-sign asks to drop signatures. Signing runs after the
    // terminal is restored, so gpg/ssh pinentry can prompt.
    let signer = if cli.no_sign {
        None
    } else {
        Some(repo::signing_config(&repository))
    };
    let report = rewrite::apply(
        &repository,
        &app.commits,
        loaded.old_tip,
        &loaded.ref_target,
        &tag_scan.tags,
        signer.as_ref(),
    )
    .context("rewriting history")?;
    print_report(&report);
    Ok(ExitCode::SUCCESS)
}

/// Run the render/read loop until the editor asks to quit.
fn run_tui(app: &mut App) -> Result<()> {
    use ratatui::crossterm::event::{self, Event, KeyEventKind};

    let mut tui = ui::Tui::enter().context("entering the terminal UI")?;
    tui.draw(app)?;
    while !app.quit {
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Release {
                let action = input::map(key, app.context());
                app.handle(action);
            }
        }
        tui.draw(app)?;
    }
    Ok(())
}

/// Print the planned author-date changes for `--dry-run`.
fn print_plan(commits: &[EditableCommit], tags: &[repo::PlannedTag]) {
    let changed = commits.iter().filter(|c| c.changed()).count();
    println!(
        "dry run: {changed} of {} commit(s) would be rewritten",
        commits.len()
    );
    for c in commits {
        let old = datetime::format(c.original.author);
        if c.changed() {
            let new = datetime::format(c.author);
            println!(
                "  {}  {old} -> {new}  {}",
                c.original.short_id, c.original.summary
            );
        } else {
            println!(
                "  {}  {old}  (unchanged)  {}",
                c.original.short_id, c.original.summary
            );
        }
    }
    // Rewriting starts at the first changed commit, so only tags from
    // there on would move. No new ids are computed on this path.
    if let Some(first) = commits.iter().position(EditableCommit::changed) {
        for t in tags.iter().filter(|t| t.commit_index >= first) {
            println!("  tag {} would move (currently {})", t.short, t.ref_oid);
        }
    }
}

/// Print the outcome of a completed rewrite.
fn print_report(report: &RewriteReport) {
    if report.count == 0 {
        println!("git-redate: no changes");
        return;
    }
    println!("git-redate: rewrote {} commit(s)", report.count);
    println!("  old tip: {}", report.old_tip);
    println!("  new tip: {}", report.new_tip);
    println!("  undo with: git reset --hard {}", report.old_tip);
    if report.resigned > 0 {
        println!("  re-signed {} commit(s)", report.resigned);
    }
    if report.dropped_signatures > 0 {
        println!(
            "  note: dropped {} signature(s) invalidated by the date change",
            report.dropped_signatures
        );
    }
    for t in &report.moved_tags {
        let sig = match t.sig {
            TagSig::Resigned => "  (re-signed)",
            TagSig::Dropped => "  (signature dropped)",
            TagSig::Unsigned => "",
        };
        println!("  moved tag {}: {} -> {}{sig}", t.name, t.old, t.new);
    }
    if !report.moved_tags.is_empty() {
        println!(
            "  note: the undo above does not restore tags; restore one with: \
             git update-ref refs/tags/<name> <old id above>"
        );
    }
}
