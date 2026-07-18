//! Terminal model: cell grid, scrollback, cursor, SGR state.
//!
//! Phase G-1.3 of `docs/plans/active/gpui-spike.md`.
//!
//! This module is intentionally framework-agnostic — no `gpui` types here
//! beyond `Hsla` for color conversion helpers. That keeps the parser
//! unit-testable without spinning up a GPUI Application (which needs a GPU
//! surface on Linux). The view layer (G-1.4) is responsible for turning
//! `Color` into `gpui::Hsla` at paint time.

use std::collections::VecDeque;
use std::path::PathBuf;

use vte::Parser;

use crate::term::perform::TermPerform;

/// A single terminal cell: character + foreground/background + attribute flags.
///
/// `Copy` + 16 bytes (ch:4 + fg:5 + bg:5 + flags:1 ≈ fits in 16). A 200×50
/// grid is ~160KB, cheap to clone for scrollback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
}

impl Cell {
    /// Blank cell with default colors and a space character.
    pub const fn blank() -> Self {
        Self {
            ch: ' ',
            fg: Color {
                kind: ColorKind::Default,
            },
            bg: Color {
                kind: ColorKind::Default,
            },
            flags: CellFlags::empty(),
        }
    }
}

/// Terminal color. `Default` lets the renderer fall back to theme defaults;
/// `Indexed` is ANSI 0-15 (we only support the 16-color base palette in
/// spike — 256-color cube is G3); `Rgb` is 24-bit truecolor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color {
    pub kind: ColorKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorKind {
    /// Use the renderer's default fg/bg.
    #[default]
    Default,
    /// ANSI 0-15 (the 16-color base palette). Values 16-255 are accepted
    /// but rendered as gray in spike — full 256-color cube is G3.
    Indexed(u8),
    /// 24-bit truecolor.
    Rgb(u8, u8, u8),
}

bitflags::bitflags! {
    /// SGR-derived cell attributes. Stored per cell so a redraw can restore
    /// the full visual state without re-running the parser.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct CellFlags: u8 {
        const BOLD      = 0b0000_0001;
        const ITALIC    = 0b0000_0010;
        const UNDERLINE = 0b0000_0100;
        const REVERSE   = 0b0000_1000;
        const DIM       = 0b0001_0000;
        const HIDDEN    = 0b0010_0000;
    }
}

#[derive(Clone, Debug)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub visible: bool,
    pub style: CursorStyle,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            visible: true,
            style: CursorStyle::Block,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

/// The terminal model. Owns the visible grid, scrollback, alt-screen, and
/// the current SGR attribute state (so newly printed cells inherit the
/// active colors/flags).
///
/// `dirty_rows` is a per-row bool array; the view (G-1.4) consults it to
/// skip unchanged rows. The parser is NOT owned here — see [`TermSession`].
pub struct TermModel {
    pub cols: usize,
    pub rows: usize,
    pub grid: Vec<Vec<Cell>>,
    pub scrollback: VecDeque<Vec<Cell>>,
    pub scrollback_cap: usize,
    pub alt_grid: Option<Vec<Vec<Cell>>>,
    pub cursor: Cursor,
    pub sgr_fg: Color,
    pub sgr_bg: Color,
    pub sgr_flags: CellFlags,
    /// Per-row dirty flag. Set by `print` / `execute` / `csi_dispatch` /
    /// `erase_*` / `line_feed` (when scrolling). View reads + clears.
    pub dirty_rows: Vec<bool>,
    /// Current working directory, updated by OSC 7 (`file://host/path`).
    /// `None` until the shell emits its first OSC 7 (bash via
    /// `PROMPT_COMMAND`, zsh via `precmd`, fish natively). The view / host
    /// can read this to drive a file manager's CWD sync — see AGENTS.md
    /// "CWD 目录跟随" hard boundary: CWD must come from the session stream,
    /// not from UI-layer polling.
    pub cwd: Option<PathBuf>,
}

/// A terminal session: a model paired with its own vte parser. The parser
/// must outlive individual `feed` calls (it holds partial escape state
/// across byte boundaries) but is cheap to keep around for the session's
/// lifetime.
///
/// The view (G-1.4) typically owns `Entity<TermSession>` and calls
/// `session.feed(bytes)` from the broadcast consumer task.
pub struct TermSession {
    pub model: TermModel,
    parser: Parser,
}

impl TermSession {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            model: TermModel::new(cols, rows),
            parser: Parser::new(),
        }
    }

    /// Convenience: feed bytes through this session's parser into its model.
    pub fn feed(&mut self, bytes: &[u8]) {
        let TermSession { model, parser } = self;
        model.feed(parser, bytes);
    }

    /// Resize the underlying model (parser is state-machine-only, no
    /// dimension awareness, so nothing to do for it).
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.model.resize(cols, rows);
    }
}

impl std::ops::Deref for TermSession {
    type Target = TermModel;
    fn deref(&self) -> &TermModel {
        &self.model
    }
}

impl std::ops::DerefMut for TermSession {
    fn deref_mut(&mut self) -> &mut TermModel {
        &mut self.model
    }
}

impl TermModel {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self::with_scrollback_cap(cols, rows, 10_000)
    }

    pub fn with_scrollback_cap(cols: usize, rows: usize, cap: usize) -> Self {
        let grid = vec![vec![Cell::blank(); cols]; rows];
        Self {
            cols,
            rows,
            grid,
            scrollback: VecDeque::with_capacity(cap),
            scrollback_cap: cap,
            alt_grid: None,
            cursor: Cursor::default(),
            sgr_fg: Color::default(),
            sgr_bg: Color::default(),
            sgr_flags: CellFlags::empty(),
            dirty_rows: vec![true; rows],
            cwd: None,
        }
    }

    /// Feed raw bytes through an externally-owned vte parser. The parser
    /// calls back into `TermPerform`, which mutates this model in place.
    ///
    /// The parser is NOT a field of `TermModel`: `Parser::advance` takes
    /// `&mut Perform` while the performer holds `&mut TermModel`, which
    /// would alias `self.parser` if it lived here. Keeping the parser in
    /// the caller ([`TermSession`]) gives the borrow checker two distinct
    /// mutable slots.
    pub fn feed(&mut self, parser: &mut Parser, bytes: &[u8]) {
        let mut perform = TermPerform { model: self };
        for &b in bytes {
            parser.advance(&mut perform, b);
        }
    }

    /// Drain and return the set of dirty row indices. The view calls this
    /// once per frame after rendering.
    pub fn take_dirty_rows(&mut self) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, d) in self.dirty_rows.iter_mut().enumerate() {
            if *d {
                *d = false;
                out.push(i);
            }
        }
        out
    }

    /// Mark every row dirty (e.g., after a resize or alt-screen swap).
    pub fn mark_all_dirty(&mut self) {
        for d in &mut self.dirty_rows {
            *d = true;
        }
    }

    /// Resize the grid. Simplified: preserves the top-left rectangle of the
    /// old grid; cursor clamped to the new bounds. Scrollback is untouched.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 {
            return;
        }
        let mut new_grid = vec![vec![Cell::blank(); cols]; rows];
        for (r, new_row) in new_grid.iter_mut().enumerate().take(self.rows.min(rows)) {
            for (c, cell) in new_row.iter_mut().enumerate().take(self.cols.min(cols)) {
                *cell = self.grid[r][c];
            }
        }
        self.grid = new_grid;
        self.cols = cols;
        self.rows = rows;
        self.cursor.row = self.cursor.row.min(rows.saturating_sub(1));
        self.cursor.col = self.cursor.col.min(cols.saturating_sub(1));
        self.dirty_rows = vec![true; rows];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: feed a string into a fresh session and return it.
    fn session_of(cols: usize, rows: usize) -> TermSession {
        TermSession::new(cols, rows)
    }

    /// Helper: read a cell from the session's underlying model.
    fn cell_at(s: &TermSession, row: usize, col: usize) -> Cell {
        s.model.grid[row][col]
    }

    #[test]
    fn feed_plain_text_advances_cursor() {
        let mut s = session_of(10, 3);
        s.feed(b"Hello");
        assert_eq!(cell_at(&s, 0, 0).ch, 'H');
        assert_eq!(cell_at(&s, 0, 1).ch, 'e');
        assert_eq!(cell_at(&s, 0, 4).ch, 'o');
        assert_eq!(s.cursor.row, 0);
        assert_eq!(s.cursor.col, 5);
    }

    #[test]
    fn feed_carriage_return_resets_col() {
        let mut s = session_of(10, 3);
        s.feed(b"ab\rX");
        assert_eq!(cell_at(&s, 0, 0).ch, 'X');
        assert_eq!(cell_at(&s, 0, 1).ch, 'b'); // not overwritten
        assert_eq!(s.cursor.col, 1);
    }

    #[test]
    fn feed_linefeed_advances_row_and_scrolls_at_bottom() {
        // Note: LF (\n) moves down a row but does NOT reset col. Use \r\n
        // for the conventional "newline resets to col 0" behavior that
        // shells emit.
        let mut s = session_of(10, 2);
        s.feed(b"L1\r\nL2");
        assert_eq!(cell_at(&s, 0, 0).ch, 'L');
        assert_eq!(cell_at(&s, 0, 1).ch, '1');
        assert_eq!(cell_at(&s, 1, 0).ch, 'L');
        assert_eq!(cell_at(&s, 1, 1).ch, '2');
        // Now at bottom row; another \r\n should scroll L1 into scrollback.
        s.feed(b"\r\nL3");
        assert_eq!(s.scrollback.len(), 1);
        assert_eq!(s.scrollback[0][0].ch, 'L');
        assert_eq!(s.scrollback[0][1].ch, '1');
        assert_eq!(cell_at(&s, 0, 0).ch, 'L');
        assert_eq!(cell_at(&s, 0, 1).ch, '2');
        assert_eq!(cell_at(&s, 1, 0).ch, 'L');
        assert_eq!(cell_at(&s, 1, 1).ch, '3');
    }

    #[test]
    fn feed_lf_only_does_not_reset_col() {
        // LF without CR: cursor moves down but keeps its column.
        let mut s = session_of(10, 3);
        s.feed(b"ab\ncd");
        assert_eq!(cell_at(&s, 0, 0).ch, 'a');
        assert_eq!(cell_at(&s, 0, 1).ch, 'b');
        // 'c' lands at (1, 2) because col was not reset by \n.
        assert_eq!(cell_at(&s, 1, 2).ch, 'c');
        assert_eq!(cell_at(&s, 1, 3).ch, 'd');
    }

    #[test]
    fn feed_sgr_red_then_reset() {
        let mut s = session_of(20, 3);
        s.feed(b"\x1b[31mRed\x1b[0mPlain");
        // "Red" should be indexed color 1 (red), "Plain" should be default.
        assert_eq!(cell_at(&s, 0, 0).ch, 'R');
        assert_eq!(cell_at(&s, 0, 0).fg.kind, ColorKind::Indexed(1));
        assert_eq!(cell_at(&s, 0, 2).fg.kind, ColorKind::Indexed(1));
        assert_eq!(cell_at(&s, 0, 3).ch, 'P');
        assert_eq!(cell_at(&s, 0, 3).fg.kind, ColorKind::Default);
        assert_eq!(s.sgr_fg.kind, ColorKind::Default);
        assert_eq!(s.sgr_flags, CellFlags::empty());
    }

    #[test]
    fn feed_sgr_bold_and_dim_reset() {
        // Sequence: BOLD, 'B', DIM, 'B','D', reset(BOLD|DIM), 'X'
        // Positions:  0=BOLD, 1=BOLD|DIM, 2=BOLD|DIM, 3=empty
        let mut s = session_of(20, 3);
        s.feed(b"\x1b[1mB\x1b[2mBD\x1b[22mX");
        assert_eq!(cell_at(&s, 0, 0).flags, CellFlags::BOLD);
        assert_eq!(cell_at(&s, 0, 1).flags, CellFlags::BOLD | CellFlags::DIM);
        assert_eq!(cell_at(&s, 0, 2).flags, CellFlags::BOLD | CellFlags::DIM);
        assert_eq!(cell_at(&s, 0, 3).flags, CellFlags::empty());
    }

    #[test]
    fn feed_sgr_truecolor_rgb_semicolon_form() {
        let mut s = session_of(20, 3);
        // 38;2;255;128;0m — semicolon-separated truecolor (what shells emit)
        s.feed(b"\x1b[38;2;255;128;0mX");
        assert_eq!(cell_at(&s, 0, 0).fg.kind, ColorKind::Rgb(255, 128, 0));
    }

    #[test]
    fn feed_sgr_truecolor_rgb_colon_subparam_form() {
        let mut s = session_of(20, 3);
        // 38:2:255:128:0m — colon-separated subparams (rare but valid)
        s.feed(b"\x1b[38:2:255:128:0mX");
        assert_eq!(cell_at(&s, 0, 0).fg.kind, ColorKind::Rgb(255, 128, 0));
    }

    #[test]
    fn feed_sgr_indexed_256_palette_semicolon() {
        let mut s = session_of(20, 3);
        // 38;5;200m — indexed 256-color, semicolon form
        s.feed(b"\x1b[38;5;200mX");
        assert_eq!(cell_at(&s, 0, 0).fg.kind, ColorKind::Indexed(200));
    }

    #[test]
    fn feed_sgr_indexed_256_palette_colon() {
        let mut s = session_of(20, 3);
        // 38:5:200m — indexed 256-color, colon subparam form
        s.feed(b"\x1b[38:5:200mX");
        assert_eq!(cell_at(&s, 0, 0).fg.kind, ColorKind::Indexed(200));
    }

    #[test]
    fn feed_csi_cup_moves_cursor() {
        let mut s = session_of(80, 24);
        s.feed(b"\x1b[5;10HX");
        assert_eq!(s.cursor.row, 4); // 0-indexed
        assert_eq!(s.cursor.col, 10); // X printed at col 9, cursor advanced to 10
        assert_eq!(cell_at(&s, 4, 9).ch, 'X');
    }

    #[test]
    fn feed_csi_ed_clears_screen() {
        let mut s = session_of(10, 3);
        s.feed(b"AAAAAAAAAA\nBBBBBBBBBB\nCCCCCCCCCC");
        s.feed(b"\x1b[2J");
        for r in 0..3 {
            for c in 0..10 {
                assert_eq!(cell_at(&s, r, c).ch, ' ');
            }
        }
    }

    #[test]
    fn feed_csi_el_clears_line() {
        let mut s = session_of(10, 3);
        s.feed(b"ABCDEFGH\r\x1b[2K");
        for c in 0..10 {
            assert_eq!(cell_at(&s, 0, c).ch, ' ');
        }
    }

    #[test]
    fn feed_alt_screen_enter_exit() {
        let mut s = session_of(10, 3);
        s.feed(b"MAIN");
        s.feed(b"\x1b[?1049hALT");
        // Alt screen: grid should be cleared, MAIN saved.
        assert_eq!(cell_at(&s, 0, 0).ch, 'A');
        assert!(s.alt_grid.is_some());
        s.feed(b"\x1b[?1049l");
        // Back to main: MAIN restored.
        assert_eq!(cell_at(&s, 0, 0).ch, 'M');
        assert_eq!(cell_at(&s, 0, 1).ch, 'A');
        assert_eq!(cell_at(&s, 0, 2).ch, 'I');
        assert_eq!(cell_at(&s, 0, 3).ch, 'N');
        assert!(s.alt_grid.is_none());
    }

    #[test]
    fn feed_cursor_visibility_decset_25() {
        let mut s = session_of(10, 3);
        assert!(s.cursor.visible);
        s.feed(b"\x1b[?25l");
        assert!(!s.cursor.visible);
        s.feed(b"\x1b[?25h");
        assert!(s.cursor.visible);
    }

    #[test]
    fn take_dirty_rows_drains_and_clears() {
        let mut s = session_of(10, 3);
        // New models start with all rows dirty (initial paint). Clear first
        // so we can isolate the effect of a single feed.
        s.model.take_dirty_rows();
        s.feed(b"X"); // dirty row 0 only
        let dirty = s.model.take_dirty_rows();
        assert_eq!(dirty, vec![0]);
        // Second call should be empty (no new writes).
        assert!(s.model.take_dirty_rows().is_empty());
    }

    #[test]
    fn resize_preserves_content_and_clamps_cursor() {
        let mut s = session_of(10, 3);
        s.feed(b"HELLO");
        s.resize(20, 5);
        assert_eq!(cell_at(&s, 0, 0).ch, 'H');
        assert_eq!(cell_at(&s, 0, 4).ch, 'O');
        assert_eq!(s.cols, 20);
        assert_eq!(s.rows, 5);
        // Cursor was at col 5; clamp to new cols-1 (19) is a no-op since 5 < 19.
        assert_eq!(s.cursor.col, 5);
    }

    #[test]
    fn osc_7_updates_cwd() {
        // OSC 7 with `file://localhost/tmp` should set cwd to `/tmp`.
        let mut s = session_of(10, 3);
        assert!(s.model.cwd.is_none());
        s.feed(b"\x1b]7;file://localhost/tmp\x07X");
        assert_eq!(s.model.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
        // Grid is not polluted: `X` after the OSC sequence prints normally.
        assert_eq!(cell_at(&s, 0, 0).ch, 'X');
    }

    #[test]
    fn osc_7_triple_slash_empty_host() {
        // `file:///home/user` (empty host) is the "localhost omitted" form.
        let mut s = session_of(10, 3);
        s.feed(b"\x1b]7;file:///home/user\x07");
        assert_eq!(
            s.model.cwd.as_deref(),
            Some(std::path::Path::new("/home/user"))
        );
    }

    #[test]
    fn osc_7_remote_host_path_kept() {
        // Remote hostname is ignored; only the path matters for CWD sync.
        let mut s = session_of(10, 3);
        s.feed(b"\x1b]7;file://example.com/var/log\x07");
        assert_eq!(
            s.model.cwd.as_deref(),
            Some(std::path::Path::new("/var/log"))
        );
    }

    #[test]
    fn osc_7_overwrites_previous_cwd() {
        // Each OSC 7 replaces the previous cwd — shells emit one per prompt.
        let mut s = session_of(10, 3);
        s.feed(b"\x1b]7;file://localhost/tmp\x07");
        assert_eq!(s.model.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
        s.feed(b"\x1b]7;file://localhost/home/user\x07");
        assert_eq!(
            s.model.cwd.as_deref(),
            Some(std::path::Path::new("/home/user"))
        );
    }

    #[test]
    fn osc_7_malformed_leaves_cwd_untouched() {
        let mut s = session_of(10, 3);
        s.feed(b"\x1b]7;file://localhost/tmp\x07");
        let cwd_before = s.model.cwd.clone();
        // Malformed payload (not a file:// URL) — cwd must stay unchanged.
        s.feed(b"\x1b]7;not-a-url\x07");
        assert_eq!(s.model.cwd, cwd_before);
        // Missing payload entirely — also a no-op.
        s.feed(b"\x1b]7;\x07");
        assert_eq!(s.model.cwd, cwd_before);
    }

    #[test]
    fn osc_7_st_terminator_also_works() {
        // Some shells use ST (ESC \) instead of BEL as the terminator.
        // vte abstracts this away — both should land in osc_dispatch.
        let mut s = session_of(10, 3);
        s.feed(b"\x1b]7;file://localhost/etc\x1b\\");
        assert_eq!(
            s.model.cwd.as_deref(),
            Some(std::path::Path::new("/etc"))
        );
    }

    #[test]
    fn osc_unknown_does_not_pollute_grid_or_cwd() {
        // Unknown OSC codes (0=title, 8=hyperlink, etc.) must be silently
        // dropped — no grid writes, no cwd changes.
        let mut s = session_of(10, 3);
        s.feed(b"\x1b]0;some title\x07X");
        assert_eq!(cell_at(&s, 0, 0).ch, 'X');
        assert!(s.model.cwd.is_none());
    }

    #[test]
    fn osc_7_split_across_feeds() {
        // OSC 7 split across two feed() calls must still parse — proves
        // the vte parser persists OSC state across byte boundaries.
        let mut s = session_of(10, 3);
        s.feed(b"\x1b]7;file://local");
        s.feed(b"host/tmp\x07");
        assert_eq!(s.model.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
    }

    #[test]
    fn feed_persists_parser_state_across_calls() {
        // A split escape sequence (prefix in one feed, suffix in another)
        // must be parsed correctly — proves the parser persists in TermSession.
        let mut s = session_of(20, 3);
        s.feed(b"\x1b[3");
        s.feed(b"1mR");
        assert_eq!(cell_at(&s, 0, 0).ch, 'R');
        assert_eq!(cell_at(&s, 0, 0).fg.kind, ColorKind::Indexed(1));
    }
}
