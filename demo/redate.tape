# vhs tape: the git-redate TUI, driven end to end against a throwaway
# repository. Regenerate with `just demo` (see docs/demo-recording.md);
# every keystroke below is scripted, so any UI change can be re-recorded
# without a human at a terminal.
#
# Flow: run `git redate HEAD~5` on five commits made between 02:11 and
# 03:40 one night; move the whole run three hours in shift mode, nudge
# the last commit on its own in single mode, expand it to pull the
# committer date back from the author date, write, then show the report
# and the new log.
#
# The order is deliberate: shift first (all five rows move at once),
# then single (one row moves), so the two modes read as a contrast. And
# every edit keeps the history in chronological order - an out-of-order
# result would look like a bug rather than a feature.

Output demo/redate.mp4

Set Theme "Catppuccin Mocha"
# 2440x980 at FontSize 36 lands on a 106x18 grid. 106 columns is the
# narrowest that still fits the report's "moved tag <old sha> -> <new
# sha>" line (104 chars) on one line; 18 rows keeps the TUI from
# floating in empty space, yet the whole story (command, tag note,
# report, new log) fits on the final screen without scrolling.
# Rendering at ~2x the 1200px README GIF keeps the downscaled text
# crisp.
Set FontSize 36
Set Width 2440
Set Height 980
Set Padding 32
Set TypingSpeed 90ms
# 12fps capture: the headless screenshots are slow at this size, and a
# higher rate drops frames and makes the clip play too fast (playback
# stays 25fps). PlaybackSpeed stays 1.0 so the Sleeps below ARE the
# pacing - tune readability here, not with a global multiplier.
Set Framerate 12
Set PlaybackSpeed 1.0

Hide
# Rebuild the repository the demo rewrites (it must be pristine on every
# render), then get a clean prompt with no cwd or git noise in it.
Type "sh /vhs/demo/make-demo-repo.sh /tmp/demo >/dev/null 2>&1 && cd /tmp/demo"
Enter
Sleep 3s
Type "export PS1='$ '; clear"
Enter
Sleep 1s
Show
Sleep 800ms

# The range: everything after HEAD~5, i.e. the five late-night commits.
Type "git redate HEAD~5"
Sleep 500ms
Enter
# The stderr note about tags lands first, then the editor takes over.
Sleep 2200ms

# Shift mode: an edit carries every newer commit with it, gaps intact.
Type "s"
Sleep 1400ms
# The cursor starts on the minute; step left onto the hour.
Type "h"
Sleep 700ms
Type "+"
Sleep 500ms
Type "+"
Sleep 500ms
Type "+"
Sleep 1800ms

# Back to single mode, where only the selected commit moves.
Type "s"
Sleep 1200ms
Type "j"
Sleep 250ms
Type "j"
Sleep 250ms
Type "j"
Sleep 250ms
Type "j"
Sleep 700ms
Type "+"
Sleep 500ms
Type "+"
Sleep 1600ms

# Expand the row to reach the committer date on its own line, and pull
# it back so the two differ.
Space
Sleep 1400ms
Type "j"
Sleep 800ms
Type "-"
Sleep 500ms
Type "-"
Sleep 1600ms
Space
Sleep 1000ms

# Write. The prompt repeats the commit count and how many tags move.
Type "w"
Sleep 1800ms
Type "y"
Sleep 2600ms

# The report: new tip, the tag that followed, the undo command - then
# the real log, in ISO form so the preserved +0800 offsets show.
Type@40ms "git log -5 --pretty='%h %ad %s' --date=iso"
Sleep 400ms
Enter
Sleep 3500ms
Hide
