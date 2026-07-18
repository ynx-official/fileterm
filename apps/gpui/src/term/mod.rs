//! Terminal subsystem: PTY bridge, model, view (G-1.x).

pub mod model;
pub mod osc;
pub mod perform;
pub mod pty;
pub mod spawn;
pub mod view;

pub use model::{Cell, CellFlags, Color, ColorKind, Cursor, CursorStyle, TermModel, TermSession};
pub use pty::{PtyHandle, TermChunk};
pub use view::TermView;
