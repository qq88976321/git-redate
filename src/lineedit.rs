//! A tiny readline-style single-line text editor.
//!
//! ratatui owns the event loop, so a full line-editing crate (rustyline,
//! reedline) that drives the terminal itself cannot be used inside the
//! TUI. This is a pure, terminal-free buffer with a cursor that the app
//! feeds one [`LineOp`] per key press; [`crate::input`] maps keys to ops
//! and [`crate::ui`] renders `text()` with a block cursor at `cursor()`.
//!
//! The cursor is a byte index that always sits on a `char` boundary, so
//! non-ASCII input is handled correctly (all slicing goes through the
//! boundary helpers, never a raw byte offset).

/// A single line of text plus a cursor position.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEditor {
    text: String,
    cursor: usize, // byte index, always on a char boundary
}

/// One editing operation, the vocabulary the input layer maps keys to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOp {
    Insert(char),
    Backspace, // delete the char before the cursor
    Delete,    // delete the char at the cursor
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
    KillWordBack, // ctrl-w: delete the word before the cursor
    KillToStart,  // ctrl-u: delete from the cursor to the start
    KillToEnd,    // ctrl-k: delete from the cursor to the end
}

impl LineOp {
    /// Whether this op can change the text (as opposed to only moving the
    /// cursor), so a caller can skip recomputing derived state on moves.
    pub fn mutates(&self) -> bool {
        matches!(
            self,
            LineOp::Insert(_)
                | LineOp::Backspace
                | LineOp::Delete
                | LineOp::KillWordBack
                | LineOp::KillToStart
                | LineOp::KillToEnd
        )
    }
}

/// A word character for word-wise motion (Unicode alphanumeric); every
/// other character is a separator.
fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

impl LineEditor {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Byte index of the char boundary immediately before `from`.
    fn prev(&self, from: usize) -> usize {
        self.text[..from]
            .chars()
            .next_back()
            .map_or(from, |c| from - c.len_utf8())
    }

    /// Byte index of the char boundary immediately after `from`.
    fn next(&self, from: usize) -> usize {
        self.text[from..]
            .chars()
            .next()
            .map_or(from, |c| from + c.len_utf8())
    }

    /// The char starting at byte index `i` (must be < len).
    fn char_at(&self, i: usize) -> char {
        self.text[i..].chars().next().unwrap()
    }

    /// Walk left from the cursor while `pred` holds for the preceding char.
    fn scan_back(&self, mut i: usize, pred: impl Fn(char) -> bool) -> usize {
        while i > 0 {
            let p = self.prev(i);
            if pred(self.char_at(p)) {
                i = p;
            } else {
                break;
            }
        }
        i
    }

    /// Walk right from the cursor while `pred` holds for the current char.
    fn scan_fwd(&self, mut i: usize, pred: impl Fn(char) -> bool) -> usize {
        while i < self.text.len() {
            if pred(self.char_at(i)) {
                i = self.next(i);
            } else {
                break;
            }
        }
        i
    }

    pub fn apply(&mut self, op: LineOp) {
        match op {
            LineOp::Insert(c) => {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            LineOp::Backspace => {
                if self.cursor > 0 {
                    let p = self.prev(self.cursor);
                    self.text.replace_range(p..self.cursor, "");
                    self.cursor = p;
                }
            }
            LineOp::Delete => {
                if self.cursor < self.text.len() {
                    let n = self.next(self.cursor);
                    self.text.replace_range(self.cursor..n, "");
                }
            }
            LineOp::Left => {
                if self.cursor > 0 {
                    self.cursor = self.prev(self.cursor);
                }
            }
            LineOp::Right => {
                if self.cursor < self.text.len() {
                    self.cursor = self.next(self.cursor);
                }
            }
            LineOp::WordLeft => {
                // Skip separators, then the word.
                let i = self.scan_back(self.cursor, |c| !is_word(c));
                self.cursor = self.scan_back(i, is_word);
            }
            LineOp::WordRight => {
                // Skip separators, then the word.
                let i = self.scan_fwd(self.cursor, |c| !is_word(c));
                self.cursor = self.scan_fwd(i, is_word);
            }
            LineOp::Home => self.cursor = 0,
            LineOp::End => self.cursor = self.text.len(),
            LineOp::KillWordBack => {
                // unix-word-rubout: back over whitespace, then non-whitespace.
                let i = self.scan_back(self.cursor, char::is_whitespace);
                let start = self.scan_back(i, |c| !c.is_whitespace());
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            LineOp::KillToStart => {
                self.text.replace_range(0..self.cursor, "");
                self.cursor = 0;
            }
            LineOp::KillToEnd => {
                self.text.truncate(self.cursor);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type a string left to right into a fresh editor.
    fn typed(s: &str) -> LineEditor {
        let mut e = LineEditor::default();
        for c in s.chars() {
            e.apply(LineOp::Insert(c));
        }
        e
    }

    #[test]
    fn insert_advances_the_cursor_to_the_end() {
        let e = typed("abc");
        assert_eq!(e.text(), "abc");
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn backspace_and_delete() {
        let mut e = typed("abc"); // cursor at 3
        e.apply(LineOp::Backspace);
        assert_eq!((e.text(), e.cursor()), ("ab", 2));
        e.apply(LineOp::Home);
        e.apply(LineOp::Delete);
        assert_eq!((e.text(), e.cursor()), ("b", 0));
        // Backspace at the start and Delete at the end are no-ops.
        e.apply(LineOp::Backspace);
        e.apply(LineOp::End);
        e.apply(LineOp::Delete);
        assert_eq!((e.text(), e.cursor()), ("b", 1));
    }

    #[test]
    fn char_motion_clamps_at_both_ends() {
        let mut e = typed("ab"); // cursor 2
        e.apply(LineOp::Right); // clamped
        assert_eq!(e.cursor(), 2);
        e.apply(LineOp::Left);
        e.apply(LineOp::Left);
        e.apply(LineOp::Left); // clamped at 0
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn word_motion_crosses_separators() {
        let mut e = typed("foo bar-baz"); // len 11, cursor 11
        e.apply(LineOp::WordLeft);
        assert_eq!(e.cursor(), 8); // start of "baz"
        e.apply(LineOp::WordLeft);
        assert_eq!(e.cursor(), 4); // start of "bar"
        e.apply(LineOp::WordLeft);
        assert_eq!(e.cursor(), 0); // start of "foo"
        e.apply(LineOp::WordRight);
        assert_eq!(e.cursor(), 3); // end of "foo"
        e.apply(LineOp::WordRight);
        assert_eq!(e.cursor(), 7); // end of "bar"
    }

    #[test]
    fn kill_word_back_removes_trailing_space_and_word() {
        let mut e = typed("foo bar "); // cursor 8
        e.apply(LineOp::KillWordBack);
        assert_eq!((e.text(), e.cursor()), ("foo ", 4));
        e.apply(LineOp::KillWordBack);
        assert_eq!((e.text(), e.cursor()), ("", 0));
    }

    #[test]
    fn kill_to_start_and_end() {
        let mut e = typed("hello world");
        e.apply(LineOp::Home);
        e.apply(LineOp::WordRight); // cursor 5 (after "hello")
        let mut a = e.clone();
        a.apply(LineOp::KillToStart);
        assert_eq!((a.text(), a.cursor()), (" world", 0));
        let mut b = e;
        b.apply(LineOp::KillToEnd);
        assert_eq!((b.text(), b.cursor()), ("hello", 5));
    }

    #[test]
    fn cursor_stays_on_char_boundaries_for_non_ascii() {
        let mut e = typed("cafe\u{301}"); // "cafe" + combining acute; last char is 2 bytes
        assert_eq!(e.text().len(), 6);
        assert_eq!(e.cursor(), 6);
        e.apply(LineOp::Left); // over the 2-byte combining mark
        assert_eq!(e.cursor(), 4);
        e.apply(LineOp::End);
        e.apply(LineOp::Backspace); // deletes the 2-byte char cleanly
        assert_eq!((e.text(), e.cursor()), ("cafe", 4));
    }

    #[test]
    fn mutates_flags_text_changing_ops_only() {
        assert!(LineOp::Insert('x').mutates());
        assert!(LineOp::Backspace.mutates());
        assert!(LineOp::KillToEnd.mutates());
        assert!(!LineOp::Left.mutates());
        assert!(!LineOp::WordRight.mutates());
        assert!(!LineOp::Home.mutates());
    }
}
