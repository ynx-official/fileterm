//! vte::Perform implementation: translates parser callbacks into model mutations.
//!
//! Phase G-1.3 of `docs/plans/active/gpui-spike.md` (CSI/SGR/execute) +
//! Phase G-1.8 (OSC 7 CWD parsing).
//!
//! Deviations from the spike skeleton:
//!   * SGR 38/48 handling. The skeleton used `p.get(1)` / `p.get(2)` on a
//!     single param slice, which only works for the colon-separated
//!     subparameter form (`38:5:X`). Real shells (bash, zsh, fish) emit the
//!     semicolon-separated form (`38;5;X`), which vte delivers as three
//!     separate `&[u16]` entries in `Params::iter()`. We handle BOTH forms:
//!     if the param containing 38 has subparams (len > 1), use them;
//!     otherwise consume the next params from the iterator.
//!   * `osc_dispatch` uses `params: &[&[u8]]` (vte 0.13 API). The skeleton
//!     had `params[0].starts_with(b"7")` which would treat OSC 7, OSC 70,
//!     OSC 700 etc. all as "OSC 7". Fixed to match the leading number
//!     precisely via `str::from_utf8` + exact `match`.
//!   * OSC 7 payload parsing lives in `crate::term::osc` (not inline here)
//!     so it can be unit-tested in isolation without constructing a
//!     `TermModel` + `Parser`. `osc_dispatch` is a thin dispatcher: parse
//!     code, delegate to `osc::parse_osc7_cwd`, assign to `model.cwd`.
//!   * Cursor movement clamps to `rows - 1` / `cols - 1` only when the
//!     dimension is non-zero; otherwise saturating math would underflow.

use vte::{Params, Perform};

use crate::term::model::{Cell, CellFlags, Color, ColorKind, TermModel};

pub struct TermPerform<'a> {
    pub model: &'a mut TermModel,
}

impl<'a> Perform for TermPerform<'a> {
    fn print(&mut self, c: char) {
        let row = self.model.cursor.row;
        let col = self.model.cursor.col;
        if row < self.model.rows && col < self.model.cols {
            self.model.grid[row][col] = Cell {
                ch: c,
                fg: self.model.sgr_fg,
                bg: self.model.sgr_bg,
                flags: self.model.sgr_flags,
            };
            self.model.dirty_rows[row] = true;
            self.model.cursor.col += 1;
            if self.model.cursor.col >= self.model.cols {
                self.model.cursor.col = 0;
                self.line_feed();
            }
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | 0x0B | 0x0C => self.line_feed(),
            b'\r' => self.model.cursor.col = 0,
            b'\x08' => {
                // BS
                if self.model.cursor.col > 0 {
                    self.model.cursor.col -= 1;
                }
            }
            b'\t' => {
                let next = (self.model.cursor.col / 8 + 1) * 8;
                let max = self.model.cols.saturating_sub(1);
                self.model.cursor.col = next.min(max);
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        match action {
            'A' => {
                // CUU
                let n = first_param(params, 1);
                self.model.cursor.row = self.model.cursor.row.saturating_sub(n as usize);
            }
            'B' => {
                // CUD
                let n = first_param(params, 1);
                let max = self.model.rows.saturating_sub(1);
                self.model.cursor.row = (self.model.cursor.row + n as usize).min(max);
            }
            'C' => {
                // CUF
                let n = first_param(params, 1);
                let max = self.model.cols.saturating_sub(1);
                self.model.cursor.col = (self.model.cursor.col + n as usize).min(max);
            }
            'D' => {
                // CUB
                let n = first_param(params, 1);
                self.model.cursor.col = self.model.cursor.col.saturating_sub(n as usize);
            }
            'H' | 'f' => {
                // CUP / HVP
                let mut iter = params.iter();
                let row = iter
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1)
                    .max(1) as usize;
                let col = iter
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1)
                    .max(1) as usize;
                let max_row = self.model.rows.saturating_sub(1);
                let max_col = self.model.cols.saturating_sub(1);
                self.model.cursor.row = (row.saturating_sub(1)).min(max_row);
                self.model.cursor.col = (col.saturating_sub(1)).min(max_col);
            }
            'J' => {
                // ED
                let mode = first_param(params, 0);
                self.erase_display(mode);
            }
            'K' => {
                // EL
                let mode = first_param(params, 0);
                self.erase_line(mode);
            }
            'm' => self.handle_sgr(params),
            'h' | 'l' => {
                // DECSET / DECRST. 1049 = alt screen, 25 = cursor visible.
                for p in params.iter() {
                    if let Some(&code) = p.first() {
                        match (action, code) {
                            ('h', 1049) => self.enter_alt_screen(),
                            ('l', 1049) => self.exit_alt_screen(),
                            ('h', 25) => self.model.cursor.visible = true,
                            ('l', 25) => self.model.cursor.visible = false,
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        // OSC sequences arrive as `params: &[&[u8]]` where params[0] is the
        // OSC code (e.g. b"7") and subsequent entries are the payload
        // split on `;`. Match the leading number precisely so OSC 7, OSC 70,
        // OSC 700 etc. don't all alias to "OSC 7".
        if params.is_empty() {
            return;
        }
        let code = std::str::from_utf8(params[0]).unwrap_or("");
        match code {
            "7" => {
                // OSC 7: CWD 跟随. Payload is `file://host/path`; vte splits
                // on `;` so params[1] is the URL. If the shell omitted the
                // `;` separator (non-conformant but seen in the wild), or
                // sent extra fields, we still try params[1] only.
                if let Some(payload) = params.get(1) {
                    if let Some(cwd) = crate::term::osc::parse_osc7_cwd(payload) {
                        self.model.cwd = Some(cwd);
                    }
                    // Parse failure leaves `cwd` untouched — see osc.rs
                    // "Robustness" section for rationale.
                }
            }
            "52" | "1337" => {
                // OSC 52 (clipboard) / 1337 (RemoteUser/RemoteCwd) — spike
                // explicitly defers these to G3 per G-1.8 step 1. Silently
                // consume so they don't pollute the grid.
            }
            _ => {
                // Unknown OSC: drop. Common ones we ignore: 0 (title),
                // 1 (icon title), 2 (set title), 8 (hyperlink), 9 (iTerm
                // growl), 104/110/111/112 (color resets).
            }
        }
    }
}

impl<'a> TermPerform<'a> {
    fn line_feed(&mut self) {
        if self.model.cursor.row + 1 >= self.model.rows {
            // Scroll: push top row into scrollback, append a blank row.
            let top = self.model.grid.remove(0);
            self.model.scrollback.push_back(top);
            if self.model.scrollback.len() > self.model.scrollback_cap {
                self.model.scrollback.pop_front();
            }
            let blank = vec![Cell::blank(); self.model.cols];
            self.model.grid.push(blank);
            for r in 0..self.model.rows {
                self.model.dirty_rows[r] = true;
            }
        } else {
            self.model.cursor.row += 1;
        }
    }

    fn erase_display(&mut self, mode: u16) {
        let blank = Cell::blank();
        match mode {
            0 => {
                let r = self.model.cursor.row;
                for c in self.model.cursor.col..self.model.cols {
                    self.model.grid[r][c] = blank;
                }
                for rr in (r + 1)..self.model.rows {
                    for c in 0..self.model.cols {
                        self.model.grid[rr][c] = blank;
                    }
                    self.model.dirty_rows[rr] = true;
                }
                self.model.dirty_rows[r] = true;
            }
            2 => {
                for r in 0..self.model.rows {
                    for c in 0..self.model.cols {
                        self.model.grid[r][c] = blank;
                    }
                    self.model.dirty_rows[r] = true;
                }
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let blank = Cell::blank();
        let r = self.model.cursor.row;
        match mode {
            0 => {
                for c in self.model.cursor.col..self.model.cols {
                    self.model.grid[r][c] = blank;
                }
            }
            2 => {
                for c in 0..self.model.cols {
                    self.model.grid[r][c] = blank;
                }
            }
            _ => {}
        }
        if r < self.model.dirty_rows.len() {
            self.model.dirty_rows[r] = true;
        }
    }

    fn handle_sgr(&mut self, params: &Params) {
        // SGR sequences come in two flavors:
        //   * Semicolon-separated: `\x1b[38;5;200m` → 3 separate params [38], [5], [200]
        //   * Colon-separated (subparams): `\x1b[38:5:200m` → 1 param [38, 5, 200]
        // Most shells emit the semicolon form. We handle both by inspecting
        // the param's subparam count: if len > 1, use subparams; else
        // consume subsequent params from the iterator.
        let mut iter = params.iter().peekable();
        while let Some(p) = iter.next() {
            let code = p.first().copied().unwrap_or(0);
            match code {
                0 => {
                    self.model.sgr_fg = Color::default();
                    self.model.sgr_bg = Color::default();
                    self.model.sgr_flags = CellFlags::empty();
                }
                1 => self.model.sgr_flags |= CellFlags::BOLD,
                2 => self.model.sgr_flags |= CellFlags::DIM,
                3 => self.model.sgr_flags |= CellFlags::ITALIC,
                4 => self.model.sgr_flags |= CellFlags::UNDERLINE,
                7 => self.model.sgr_flags |= CellFlags::REVERSE,
                8 => self.model.sgr_flags |= CellFlags::HIDDEN,
                22 => self.model.sgr_flags &= !(CellFlags::BOLD | CellFlags::DIM),
                23 => self.model.sgr_flags &= !CellFlags::ITALIC,
                24 => self.model.sgr_flags &= !CellFlags::UNDERLINE,
                27 => self.model.sgr_flags &= !CellFlags::REVERSE,
                28 => self.model.sgr_flags &= !CellFlags::HIDDEN,
                30..=37 => {
                    self.model.sgr_fg = Color {
                        kind: ColorKind::Indexed(code as u8 - 30),
                    }
                }
                38 => {
                    if let Some(c) = parse_color_param(p, &mut iter) {
                        self.model.sgr_fg = c;
                    }
                }
                39 => self.model.sgr_fg = Color::default(),
                40..=47 => {
                    self.model.sgr_bg = Color {
                        kind: ColorKind::Indexed(code as u8 - 40),
                    }
                }
                48 => {
                    if let Some(c) = parse_color_param(p, &mut iter) {
                        self.model.sgr_bg = c;
                    }
                }
                49 => self.model.sgr_bg = Color::default(),
                90..=97 => {
                    self.model.sgr_fg = Color {
                        kind: ColorKind::Indexed((code - 90 + 8) as u8),
                    }
                }
                100..=107 => {
                    self.model.sgr_bg = Color {
                        kind: ColorKind::Indexed((code - 100 + 8) as u8),
                    }
                }
                _ => {}
            }
        }
    }

    fn enter_alt_screen(&mut self) {
        if self.model.alt_grid.is_none() {
            self.model.alt_grid = Some(std::mem::replace(
                &mut self.model.grid,
                vec![vec![Cell::blank(); self.model.cols]; self.model.rows],
            ));
            self.model.dirty_rows = vec![true; self.model.rows];
            self.model.cursor.row = 0;
            self.model.cursor.col = 0;
        }
    }

    fn exit_alt_screen(&mut self) {
        if let Some(alt) = self.model.alt_grid.take() {
            self.model.grid = alt;
            self.model.dirty_rows = vec![true; self.model.rows];
            self.model.cursor.row = 0;
            self.model.cursor.col = 0;
        }
    }
}

/// Read the first parameter value, defaulting to `default` if absent or zero.
/// (Most CSI sequences treat a missing param as "1", but `J`/`K`/etc. treat
/// it as "0". Caller passes the right default.)
fn first_param(params: &Params, default: u16) -> u16 {
    params
        .iter()
        .next()
        .and_then(|p| p.first())
        .copied()
        .filter(|&v| v != 0)
        .unwrap_or(default)
}

/// Parse an SGR 38/48 color parameter. Handles both forms:
///   * `p = [38, 5, IDX]` (colon subparams) — read p[1], p[2]
///   * `p = [38]` then iterator yields [5], [IDX] — consume from iter
///
/// Returns `None` if the form is unrecognized (parser leaves SGR state
/// untouched, matching xterm's behavior for malformed sequences).
fn parse_color_param<'a, I>(p: &'a [u16], iter: &mut std::iter::Peekable<I>) -> Option<Color>
where
    I: Iterator<Item = &'a [u16]>,
{
    if p.len() >= 3 {
        // Colon-separated subparams: [38, 5, IDX] or [38, 2, R, G, B]
        match p.get(1)? {
            5 => Some(Color {
                kind: ColorKind::Indexed(*p.get(2)? as u8),
            }),
            2 => Some(Color {
                kind: ColorKind::Rgb(*p.get(2)? as u8, *p.get(3)? as u8, *p.get(4)? as u8),
            }),
            _ => None,
        }
    } else {
        // Semicolon-separated: consume next param(s) from iterator.
        let next = iter.next()?;
        match next.first()? {
            5 => {
                let idx = iter.next()?.first().copied()? as u8;
                Some(Color {
                    kind: ColorKind::Indexed(idx),
                })
            }
            2 => {
                let r = iter.next()?.first().copied()? as u8;
                let g = iter.next()?.first().copied()? as u8;
                let b = iter.next()?.first().copied()? as u8;
                Some(Color {
                    kind: ColorKind::Rgb(r, g, b),
                })
            }
            _ => None,
        }
    }
}
