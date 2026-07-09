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
use git_redate::rewrite::RewriteReport;
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

    let editable: Vec<EditableCommit> = loaded
        .commits
        .into_iter()
        .map(EditableCommit::new)
        .collect();
    let mut app = App::new(editable, effective.mode, cli.dry_run, cli.separate);

    if cli.dry_run {
        // Edit interactively when possible, then print the plan without
        // writing anything. Headless, this just previews the range.
        if std::io::stdout().is_terminal() {
            run_tui(&mut app).context("running the editor")?;
        }
        print_plan(&app.commits);
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
fn print_plan(commits: &[EditableCommit]) {
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
}
