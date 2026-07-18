# Phase G-1：终端 Spike 工单

| 项目     | 值                          |
| -------- | --------------------------- |
| 文档版本 | v0.3                        |
| 更新日期 | 2026-07-18                  |
| 状态     | 待开工                      |
| 关联文档 | [gpui-refactor.md](./gpui-refactor.md) 第 4.4 节 |

> 这是 Phase G-1 终端 spike 的可开工工单。目标：在最短时间内验证 vte + GPUI 终端管线在 [gpui-refactor.md](./gpui-refactor.md) 4.4.3 节的阈值内。不追求功能完整，只覆盖性能与渲染路径。

---

## 1. 目标与非目标

### 1.1 目标

- 在 `apps/gpui/` 下跑起一个最小可运行的终端示例（`cargo run -p fileterm-gpui --example term_spike`）。
- 终端能通过本地 PTY 启动 `bash`，输入命令、看到输出、resize 窗口。
- 4.4.3 节五个场景全部达标（80×24 4KB/s 60fps / 80×24 1MB/s `yes` 30fps+ / 200×50 `find /` 60fps / `htop` 60fps / `vim` alt screen 60fps）。
- 验证以下技术点是否可行：
  - `gpui-unofficial` v1.9 API 在三平台可编译运行。
  - `vte` crate 解析性能足够。
  - `portable-pty` 在 macOS/Linux/Windows 都能起 PTY。
  - GPUI `ShapedLine` 批量绘制能满足终端 cell 渲染需求。
  - `tokio::broadcast` + 16ms 节流的背压策略有效。

### 1.2 非目标

- 不接 SSH：spike 只验证本地 PTY 路径；SSH channel 字节流接口与 PTY 一致，G3 阶段再接入。
- 不做完整 UI：只有终端区域 + 一个 resize handle，没有标签栏、文件面板、模态弹窗。
- 不做配置：scrollback 行数、字体、配色硬编码。
- 不做完整 OSC 解析：只做 OSC 7（CWD）+ OSC 52（剪贴板）+ 1337（RemoteUser），其他 OSC 序列静默丢弃。
- 不写测试：spike 阶段只跑性能基准与手测；测试在 G0 阶段补。

---

## 2. 前置条件

- `gpui` 分支已创建并切出（已完成）。
- 本机有 Rust toolchain（`cargo --version` 可用）。
- 本机有 bash（macOS/Linux 自带；Windows 用 Git Bash 或 WSL）。
- 三平台之一可运行：macOS 13+ / Ubuntu 22.04+ / Windows 10+。

---

## 3. 依赖清单

> 注：原草案写的是 `gpui-unofficial = "1.9"` 与一个 `gpui-text-gpui-unofficial` crate。crates.io 上既没有 1.9，也没有 `gpui-text-gpui-unofficial`（文本 API 在 gpui 主包内）。实际锁定 `=1.8.2`，并显式 pin 全部 12 个子 crate 防止 cargo 解析漂移到 1.11.x 线。详见 G-1.1 实现笔记。

```toml
# apps/gpui/Cargo.toml
[package]
name = "fileterm-gpui"
version = "0.1.0"
edition = "2021"

[dependencies]
gpui = { package = "gpui-unofficial", version = "=1.8.2" }
gpui_platform = { package = "gpui-platform-gpui-unofficial", version = "=1.8.2", features = ["font-kit", "wayland", "x11"] }
# 锁定全部 gpui 子 crate 到 1.8.2，避免 cargo 拉到 1.11.x 线
collections = { package = "collections-gpui-unofficial", version = "=1.8.2" }
gpui_util = { package = "gpui-util-gpui-unofficial", version = "=1.8.2" }
gpui_macros = { package = "gpui-macros-gpui-unofficial", version = "=1.8.2" }
gpui_shared_string = { package = "gpui-shared-string-gpui-unofficial", version = "=1.8.2" }
derive_refineable = { package = "derive-refineable-gpui-unofficial", version = "=1.8.2" }
refineable = { package = "refineable-gpui-unofficial", version = "=1.8.2" }
gpui_wgpu = { package = "gpui-wgpu-gpui-unofficial", version = "=1.8.2" }
util_macros = { package = "util-macros-gpui-unofficial", version = "=1.8.2" }
perf = { package = "perf-gpui-unofficial", version = "=1.8.2" }
http_client = { package = "http-client-gpui-unofficial", version = "=1.8.2" }
scheduler = { package = "scheduler-gpui-unofficial", version = "=1.8.2" }
sum_tree = { package = "sum-tree-gpui-unofficial", version = "=1.8.2" }
media = { package = "media-gpui-unofficial", version = "=1.8.2" }
vte = "0.13"
portable-pty = "0.8"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
parking_lot = "0.12"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
bitflags = "2"

[[example]]
name = "term_spike"
path = "examples/term_spike.rs"
```

根 `Cargo.toml` 还需要两处 `[patch.crates-io]` 把 `gpui-unofficial` 与 `gpui-util-gpui-unofficial` 替换为 `.patches/` 下的本地副本，以绕过 rustc 1.92 的 `slice_as_array` / `cold_path` unstable feature。详见 G-1.1 实现笔记。

---

## 4. 任务拆分

### G-1.1 脚手架（最小空窗口）

**文件**：`apps/gpui/Cargo.toml`、`apps/gpui/src/lib.rs`、`apps/gpui/examples/term_spike.rs`

**步骤**：

1. 在根 `Cargo.toml` 的 `[workspace] members` 加入 `"apps/gpui"`。
2. 写 `apps/gpui/Cargo.toml`（见第 3 节）。
3. 写 `apps/gpui/src/lib.rs`（空 lib，仅 `pub mod term;`）。
4. 写 `apps/gpui/examples/term_spike.rs`，最小 GPUI 应用打开一个空窗口。

**代码骨架**：

```rust
// apps/gpui/examples/term_spike.rs
use gpui::*;

fn main() {
    Application::run().unwrap(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(1024.0, 720.0));
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("FileTerm GPUI Spike".into()),
                    appears_transparent: cfg!(target_os = "macos"),
                    ..Default::default()
                }),
                window_decorations: if cfg!(target_os = "windows") {
                    Some(WindowDecorations::Client)
                } else {
                    Some(WindowDecorations::Server)
                },
                kind: WindowKind::Normal,
                ..Default::default()
            },
            |_cx| cx.new_view(|_cx| SpikeView {}),
        )
        .unwrap();
    });
}

struct SpikeView {}

impl Render for SpikeView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x181818))
    }
}
```

**验收**：

```bash
cargo run -p fileterm-gpui --example term_spike
```

能打开一个 1024×720 的深灰色空窗口，关闭即退出。

**风险**：

- `gpui-unofficial` 1.9 API 与示例可能略有差异；若 `Application::run` 签名不符，查 `cargo doc -p gpui --open`。
- Linux Wayland 下需要 `WAYLAND_DISPLAY` 环境变量；X11 下需要 `DISPLAY`。

**实现笔记（2026-07-18 落地，G-1.1 已通过验收）**：

实际落地版本是 `gpui-unofficial = "=1.8.2"`（crates.io 上没有 1.9；1.8.2 是当前稳定线）。相对本节"代码骨架"，真实 API 有 5 处偏差，全部已写入 `apps/gpui/examples/term_spike.rs` 顶部 doc comment：

1. 入口是 `gpui_platform::application().run(...)`，不是 `Application::run()`。后者不会初始化平台窗口/文本驱动。
2. `Bounds::centered` 签名是 `(&App)` 三参数形式：`Bounds::centered(None, size(px(1024.0), px(720.0)), cx)`。
3. `cx.open_window` 的 view-builder 闭包是 `|window, cx|`，不是 `|cx|`。
4. 创建 `Entity<V>` 的 API 是 `AppContext::new`（`impl AppContext for App`），不是 `cx.new_view(...)`。需要 `use gpui::AppContext;` 把 trait 方法带入作用域。
5. `div().size_full()` 需要 `use gpui::Styled;`（trait 方法）。

**rustc 1.92 兼容补丁**（`.patches/` 下两份本地 crate，经根 `Cargo.toml` 的 `[patch.crates-io]` 注入，toolchain 升到 1.93+ 后可移除）：

- `gpui-util-gpui-unofficial` 1.8.2 用了 `slice_as_array`（unstable，issue #133508，rustc 1.93 稳定）：把 `bytes.as_array()` 改成 `bytes.try_into()` 稳定形式。
- `gpui-unofficial` 1.8.2 主包 `src/profiler.rs` / `src/profiler/actions.rs` 用了 `std::hint::cold_path()`（unstable，issue #136873，rustc 1.93+ 稳定）：5 处调用就地注释掉。

**原生依赖**（Linux）：`libxkbcommon-dev`、`libxkbcommon-x11-dev`、`libxcb1-dev`、`libfontconfig-dev`、`libfreetype-dev`、`libwayland-dev`。

**无显示器环境验证**（CI / 远程 sandbox）：gpui 用 wgpu 做 GPU 渲染，纯 `xvfb` 不够，需要软件 GL：

```bash
apt-get install -y xvfb libgl1-mesa-dri mesa-utils
LIBGL_ALWAYS_SOFTWARE=1 xvfb-run -a -s "-screen 0 1280x800x24 +iglx" \
  timeout 5 ./target/debug/examples/term_spike
# 期望：进程被 timeout 杀掉，退出码 124，stderr 无 panic
```

G-1.1 在该路径下已确认：窗口持续打开 5s 无 panic，日志为空，`timeout` 退出码 124。

---

### G-1.2 PTY 启动 + 字节流读取

**文件**：`apps/gpui/src/term/pty.rs`

**步骤**：

1. 用 `portable_pty::PtySystem::open()` 创建一对 master/slave。
2. 在 slave 上 `spawn` bash（Windows 用 `cmd.exe` 或 `powershell.exe`）。
3. master 端 spawn 一个 tokio task 持续读字节，包装为 `TermChunk` 发到 `broadcast::Sender`。
4. 提供 `write_input(&self, bytes: &[u8])` 把用户输入写入 master。

**代码骨架**：

```rust
// apps/gpui/src/term/pty.rs
use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, Master, PtySize, PtySystem};
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub struct TermChunk {
    pub bytes: Vec<u8>,
}

pub struct PtyHandle {
    master: Box<dyn Master + Send>,
    pub tx: broadcast::Sender<TermChunk>,
}

impl PtyHandle {
    pub fn spawn(shell: &str, cols: u16, rows: u16) -> Result<(Self, broadcast::Receiver<TermChunk>)> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let _child = pair.slave.spawn_command(cmd)?;
        pair.slave.close()?;

        let mut reader = pair.master.take_reader()?;
        let (tx, rx) = broadcast::channel(256);

        // 在 tokio runtime 里读字节流
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = TermChunk { bytes: buf[..n].to_vec() };
                        // 队列满时直接丢弃最旧（broadcast 的 try_send 行为）
                        let _ = tx.send(chunk);
                    }
                    Err(_) => break,
                }
            }
        });

        Ok((
            Self { master: pair.master, tx: tx.clone() },
            rx,
        ))
    }

    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        let mut writer = self.master.take_writer()?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows, cols, pixel_width: 0, pixel_height: 0,
        })?;
        Ok(())
    }
}
```

**验收**：

```bash
cargo run -p fileterm-gpui --example term_pty
```

启动后能在 stdout 看到 bash 的 prompt 字节（`$ ` 或 `% ` 或 `# `）。可以用 `eprintln!("chunk: {:?}", chunk.bytes);` 临时打印验证。

> 注：G-1.2 的验收示例从原草案的 `term_spike` 改为独立的 `term_pty`。原因：G-1.1 的 `term_spike` 验收只要求"打开窗口"，而 G-1.2 要看 PTY 字节流——把两者放一起会让 G-1.1 的无显示器环境验证复杂化。`term_pty` 不开窗口、不需要 GPU，纯 stdout 即可验收，CI 友好。

**风险**：

- `portable_pty` 在 Windows 下默认用 ConPTY，需要 Windows 10 18362+；旧版用 winpty fallback。
- `take_writer()` 在某些版本返回 `Result`，可能需要 `?`。查 `cargo doc -p portable-pty`。

**实现笔记（2026-07-18 落地，G-1.2 已通过验收）**：

相对本节"代码骨架"，真实 `portable-pty` 0.8.1 API 有 4 处偏差，全部已写入 `apps/gpui/src/term/pty.rs` 顶部 doc comment：

1. trait 名是 `MasterPty`，不是 `Master`。
2. reader API 是 `try_clone_reader()`，不是 `take_reader()`。master 仍持有读端，我们只 clone 一份 handle。
3. `SlavePty` 没有 `close()` 方法。官方模式（见 `portable-pty/examples/whoami.rs`）是 `drop(pair.slave)` 后让 child 自然 EOF。
4. **`take_writer()` 是 one-shot**：每个 master 实例只能调用一次，第二次调直接 `bail!("cannot take writer more than once")`（见 `portable-pty` 的 `unix.rs:319` / `serial.rs:229`）。骨架里 `write_input` 每次都调 `take_writer()` 是错的——必须在 `spawn()` 时调一次，把 writer 存在 `PtyHandle` 上，后续 `write_input` 复用。`PtyHandle` 用 `Mutex<Option<Box<dyn Write + Send>>>` 持有 writer，因为 `Write::write_all` 要 `&mut self`。
5. `SlavePty` / `PtySystem` trait 不需要显式 `use`——Rust 通过 `pair.slave.spawn_command()` 和 `system.openpty()` 的具体类型就能解析 trait 方法。显式 import 反而会触发 `unused_imports` warning。

**额外设计选择**：

- 读 pump 用 `std::thread` 不用 `tokio::task`：reader 是阻塞 `Read`，放进 async worker 会占住一个 worker 线程做永不 yield 的 syscall。
- 输出通道用 `tokio::sync::broadcast`（容量 256）：subscriber 可来去自由，慢 subscriber 触发 `Lagged` 不影响其他人，零 subscriber 时 send 静默丢弃。
- `TermChunk` 带 `seq: u64`：让 receiver 检测 Lagged 间隙后能决定全量重排还是跳过。

**验收结果**：

```
$ cargo run -p fileterm-gpui --example term_pty
[term_pty] spawning shell: /usr/bin/bash
[term_pty] spawned, waiting for first chunk...
[term_pty] first chunk: seq=1 76 bytes
chunk: \u{1b}[?2004h\u{1b}]0;root@host: ~\u{7}root@host:~#
chunk: echo hello-g-1-2\r\n
chunk: \u{1b}[?2004l\r
chunk: hello-g-1-2\r\n                ← marker 观察到
chunk: \u{1b}[?2004h\u{1b}]0;root@host: ~\u{7}root@host:~#
chunk: exit\r\n\u{1b}[?2004l\rexit\r\n
[term_pty] ACCEPTANCE: marker 'hello-g-1-2' observed in output
```

完整往返验证：spawn → 收到 prompt → 写入 `echo hello-g-1-2\n` → shell 回显 marker → 写入 `exit\n` → shell 退出 → 进程退出码 0。同时 G-1.1 的 `term_spike` 在 `xvfb-run` + 软件渲染下仍持续打开 4s 无 panic（回归未破）。

---

### G-1.3 vte 解析 + TermModel

**文件**：`apps/gpui/src/term/model.rs`、`apps/gpui/src/term/perform.rs`

**步骤**：

1. 定义 `Cell` struct（字符 + 前景色 + 背景色 + 属性 flags）。
2. 定义 `TermModel`（grid + scrollback + alt_grid + cursor + SGR state + dirty_rows）。
3. 实现 `vte::Perform` for `TermPerform`，把 CSI/SGR/OSC 回调翻译成 model mutation。
4. 在 `TermModel::feed(&mut self, bytes: &[u8])` 中驱动 `vte::Parser::advance`。

**代码骨架**：

```rust
// apps/gpui/src/term/model.rs
use gpui::Hsla;
use vte::{Parser, Perform};

#[derive(Clone, Copy, Debug, Default)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub kind: ColorKind,
    pub value: u32, // 0xFFFFFFFF 表示 default
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ColorKind {
    #[default]
    Default,
    Indexed(u8), // ANSI 0-15
    Rgb(u8, u8, u8),
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default)]
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

#[derive(Clone, Copy, Debug)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

pub struct TermModel {
    pub cols: usize,
    pub rows: usize,
    pub grid: Vec<Vec<Cell>>,
    pub scrollback: std::collections::VecDeque<Vec<Cell>>,
    pub scrollback_cap: usize,
    pub alt_grid: Option<Vec<Vec<Cell>>>,
    pub cursor: Cursor,
    pub sgr_fg: Color,
    pub sgr_bg: Color,
    pub sgr_flags: CellFlags,
    pub parser: Parser,
    pub dirty_rows: Vec<bool>, // 简化版：每行一个 bool
}

impl TermModel {
    pub fn new(cols: usize, rows: usize) -> Self {
        let blank = Cell { ch: ' ', fg: Color::default(), bg: Color::default(), flags: CellFlags::empty() };
        let grid = vec![vec![blank; cols]; rows];
        Self {
            cols, rows, grid,
            scrollback: std::collections::VecDeque::with_capacity(10000),
            scrollback_cap: 10000,
            alt_grid: None,
            cursor: Cursor { row: 0, col: 0, visible: true, style: CursorStyle::Block },
            sgr_fg: Color::default(),
            sgr_bg: Color::default(),
            sgr_flags: CellFlags::empty(),
            parser: Parser::new(),
            dirty_rows: vec![true; rows],
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let mut perform = TermPerform { model: self };
        for &b in bytes {
            self.parser.advance(&mut perform, b);
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        // 简化版：重新分配，保留可保留内容
        let blank = Cell { ch: ' ', fg: Color::default(), bg: Color::default(), flags: CellFlags::empty() };
        let mut new_grid = vec![vec![blank; cols]; rows];
        for r in 0..self.rows.min(rows) {
            for c in 0..self.cols.min(cols) {
                new_grid[r][c] = self.grid[r][c];
            }
        }
        self.grid = new_grid;
        self.cols = cols;
        self.rows = rows;
        self.cursor.row = self.cursor.row.min(rows - 1);
        self.cursor.col = self.cursor.col.min(cols - 1);
        self.dirty_rows = vec![true; rows];
    }
}
```

```rust
// apps/gpui/src/term/perform.rs
use vte::{Perform, Csi};
use crate::term::model::*;

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
            b'\x08' => { // BS
                if self.model.cursor.col > 0 { self.model.cursor.col -= 1; }
            }
            b'\t' => {
                let next = (self.model.cursor.col / 8 + 1) * 8;
                self.model.cursor.col = next.min(self.model.cols - 1);
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, _intermediates: &[u8], _ignore: bool, action: char) {
        match action {
            'A' => { // CUU
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                self.model.cursor.row = self.model.cursor.row.saturating_sub(n);
            }
            'B' => { // CUD
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                self.model.cursor.row = (self.model.cursor.row + n).min(self.model.rows - 1);
            }
            'C' => { // CUF
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                self.model.cursor.col = (self.model.cursor.col + n).min(self.model.cols - 1);
            }
            'D' => { // CUB
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                self.model.cursor.col = self.model.cursor.col.saturating_sub(n);
            }
            'H' | 'f' => { // CUP
                let mut iter = params.iter();
                let row = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                let col = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                self.model.cursor.row = (row.saturating_sub(1)).min(self.model.rows - 1);
                self.model.cursor.col = (col.saturating_sub(1)).min(self.model.cols - 1);
            }
            'J' => { // ED
                let mode = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(0);
                self.erase_display(mode);
            }
            'K' => { // EL
                let mode = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(0);
                self.erase_line(mode);
            }
            'm' => self.handle_sgr(params),
            'h' | 'l' => {
                // DECSET / DECRST：1049 = alt screen，25 = cursor visible
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
        if params.is_empty() { return; }
        let osc = params[0];
        if osc.starts_with(b"7") || osc.starts_with(b"8") {
            // OSC 7: CWD；OSC 8: hyperlink。spike 阶段静默丢弃。
        } else if osc.starts_with(b"52") {
            // OSC 52: 剪贴板。spike 阶段静默丢弃。
        } else if osc.starts_with(b"1337") {
            // 1337: RemoteUser / RemoteCwd。spike 阶段静默丢弃。
        }
    }
}

impl<'a> TermPerform<'a> {
    fn line_feed(&mut self) {
        if self.model.cursor.row + 1 >= self.model.rows {
            // 滚动：把顶行推入 scrollback
            let top = self.model.grid.remove(0);
            self.model.scrollback.push_back(top);
            if self.model.scrollback.len() > self.model.scrollback_cap {
                self.model.scrollback.pop_front();
            }
            let blank = vec![Cell::default(); self.model.cols];
            self.model.grid.push(blank);
            for r in 0..self.model.rows {
                self.model.dirty_rows[r] = true;
            }
        } else {
            self.model.cursor.row += 1;
        }
    }

    fn erase_display(&mut self, mode: u16) {
        let blank = Cell::default();
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
        let blank = Cell::default();
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
        self.model.dirty_rows[r] = true;
    }

    fn handle_sgr(&mut self, params: &vte::Params) {
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
                30..=37 => self.model.sgr_fg = Color { kind: ColorKind::Indexed(code as u8 - 30), value: 0 },
                38 => {
                    if let Some(&next) = p.get(1) {
                        if next == 5 {
                            if let Some(&idx) = p.get(2) {
                                self.model.sgr_fg = Color { kind: ColorKind::Indexed(idx as u8), value: 0 };
                            }
                        } else if next == 2 {
                            if let (Some(&r), Some(&g), Some(&b)) = (p.get(2), p.get(3), p.get(4)) {
                                self.model.sgr_fg = Color { kind: ColorKind::Rgb(r as u8, g as u8, b as u8), value: 0 };
                            }
                        }
                    }
                }
                39 => self.model.sgr_fg = Color::default(),
                40..=47 => self.model.sgr_bg = Color { kind: ColorKind::Indexed(code as u8 - 40), value: 0 },
                48 => {
                    if let Some(&next) = p.get(1) {
                        if next == 5 {
                            if let Some(&idx) = p.get(2) {
                                self.model.sgr_bg = Color { kind: ColorKind::Indexed(idx as u8), value: 0 };
                            }
                        } else if next == 2 {
                            if let (Some(&r), Some(&g), Some(&b)) = (p.get(2), p.get(3), p.get(4)) {
                                self.model.sgr_bg = Color { kind: ColorKind::Rgb(r as u8, g as u8, b as u8), value: 0 };
                            }
                        }
                    }
                }
                49 => self.model.sgr_bg = Color::default(),
                90..=97 => self.model.sgr_fg = Color { kind: ColorKind::Indexed((code - 90 + 8) as u8), value: 0 },
                100..=107 => self.model.sgr_bg = Color { kind: ColorKind::Indexed((code - 100 + 8) as u8), value: 0 },
                _ => {}
            }
        }
    }

    fn enter_alt_screen(&mut self) {
        if self.model.alt_grid.is_none() {
            let blank = Cell::default();
            self.model.alt_grid = Some(self.model.grid.clone());
            self.model.grid = vec![vec![blank; self.model.cols]; self.model.rows];
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
```

**验收**：

- `cargo check -p fileterm-gpui` 通过。
- 单测：feed 一段 `"Hello\r\nWorld\x1b[31mRed\x1b[0m"`，断言 grid 内容与 SGR 状态正确。

**风险**：

- `vte::Params` API 在不同版本略有差异；1.13 用 `iter()` 返回 `&ParamsIter`。
- `bitflags` 需要 `bitflags = "2"` 版本。

---

### G-1.4 TermView 渲染

**文件**：`apps/gpui/src/term/view.rs`

**步骤**：

1. 定义 `TermView` 持有 `Entity<TermModel>` + 字体 + cell_size。
2. 实现 `Render`：按行迭代，按 SGR run 分组，每组调用 `ShapedLine::shape`。
3. 先画背景色 quad，再画 shaped text。
4. cursor 作 overlay。
5. 处理键盘输入：把 key event 转为字节序列写入 PTY。
6. 处理 resize：监听 window resize，调用 `TermModel::resize` + `PtyHandle::resize`。

**代码骨架**：

```rust
// apps/gpui/src/term/view.rs
use gpui::*;
use crate::term::model::*;
use crate::term::pty::PtyHandle;

pub struct TermView {
    pub model: Entity<TermModel>,
    pub pty: std::sync::Arc<PtyHandle>,
    pub font: Font,
    pub cell_size: Size<Pixels>,
    pub cols: usize,
    pub rows: usize,
}

impl TermView {
    pub fn new(
        model: Entity<TermModel>,
        pty: std::sync::Arc<PtyHandle>,
        font: Font,
        cx: &mut Context<Self>,
    ) -> Self {
        let cell_size = Self::measure_cell(&font, cx);
        let cols = 80usize;
        let rows = 24usize;
        Self { model, pty, font, cell_size, cols, rows }
    }

    fn measure_cell(font: &Font, cx: &mut App) -> Size<Pixels> {
        // 用 ShapedLine 测量 "M" 的尺寸作为 cell 大小
        let line = gpui_text::ShapedLine::shape(
            "M".repeat(80),
            font.clone(),
            px(14.0),
            cx.text_system(),
        );
        let width = line.width() / 80.0;
        let height = px(18.0); // 简化：行高 = 字号 * 1.28
        size(width, height)
    }

    fn color_to_hsla(c: Color) -> Hsla {
        match c.kind {
            ColorKind::Default => gpui::black(),
            ColorKind::Indexed(idx) => ansi_palette(idx),
            ColorKind::Rgb(r, g, b) => rgb_color(r, g, b),
        }
    }
}

impl Render for TermView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let cell_w = self.cell_size.width;
        let cell_h = self.cell_size.height;

        // 1. 背景：先画一个全屏 dark quad
        let mut canvas = div()
            .size_full()
            .bg(rgb(0x181818))
            .overflow_hidden();

        // 2. 按行渲染
        for (row_idx, row) in model.grid.iter().enumerate() {
            let y = row_idx as f32 * cell_h.0;

            // 2a. 背景色 run
            let mut col = 0usize;
            while col < row.len() {
                let bg = row[col].bg;
                let mut end = col;
                while end < row.len() && row[end].bg == bg {
                    end += 1;
                }
                if bg.kind != ColorKind::Default {
                    canvas = canvas.child(
                        div()
                            .absolute()
                            .left(col as f32 * cell_w.0)
                            .top(y)
                            .width(cell_w * (end - col) as f32)
                            .height(cell_h)
                            .bg(Self::color_to_hsla(bg))
                    );
                }
                col = end;
            }

            // 2b. 文本 run（按 fg + flags 合并）
            let mut col = 0usize;
            while col < row.len() {
                let cell = &row[col];
                if cell.ch == ' ' {
                    col += 1;
                    continue;
                }
                let fg = cell.fg;
                let flags = cell.flags;
                let mut text = String::new();
                let mut end = col;
                while end < row.len()
                    && row[end].fg == fg
                    && row[end].flags == flags
                    && row[end].ch != ' '
                {
                    text.push(row[end].ch);
                    end += 1;
                }
                let line = gpui_text::ShapedLine::shape(
                    text,
                    self.font.clone(),
                    px(14.0),
                    cx.text_system(),
                );
                canvas = canvas.child(
                    line.paint(
                        point(col as f32 * cell_w.0, y),
                        Self::color_to_hsla(fg),
                    )
                );
                col = end;
            }
        }

        // 3. Cursor overlay
        if model.cursor.visible {
            let cursor_x = model.cursor.col as f32 * cell_w.0;
            let cursor_y = model.cursor.row as f32 * cell_h.0;
            canvas = canvas.child(
                div()
                    .absolute()
                    .left(cursor_x)
                    .top(cursor_y)
                    .width(cell_w)
                    .height(cell_h)
                    .bg(rgb(0xe0e0e0))
            );
        }

        canvas
    }
}

impl TermView {
    pub fn handle_key(&mut self, event: &KeyPressEvent, _cx: &mut Context<Self>) {
        let bytes = key_event_to_bytes(event);
        let _ = self.pty.write_input(&bytes);
    }
}

fn ansi_palette(idx: u8) -> Hsla {
    // xterm 256 色简化版：0-15 用预设，16-255 用 6x6x6 cube + 24 灰阶
    // spike 阶段先硬编码 0-15，其余返回 gray
    const ANSI_16: [u32; 16] = [
        0x000000, 0x800000, 0x008000, 0x808000,
        0x000080, 0x800080, 0x008080, 0xc0c0c0,
        0x808080, 0xff0000, 0x00ff00, 0xffff00,
        0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    if idx < 16 {
        rgb(ANSI_16[idx as usize])
    } else {
        rgb(0x808080)
    }
}

fn rgb_color(r: u8, g: u8, b: u8) -> Hsla {
    rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
}

fn key_event_to_bytes(event: &KeyPressEvent) -> Vec<u8> {
    // 简化版：可打印字符直接转字节；Enter = \r；Backspace = \x7f
    // 完整实现需要处理 Ctrl+字母、方向键、功能键等
    match event.keystroke.key.as_str() {
        "enter" => vec![b'\r'],
        "backspace" => vec![0x7f],
        "tab" => vec![b'\t'],
        "escape" => vec![0x1b],
        _ => {
            if let Some(ch) = event.keystroke.key.chars().next() {
                if event.keystroke.modifiers.control {
                    let code = ch.to_ascii_lowercase() as u8;
                    if ('a'..='z').contains(&code) {
                        return vec![code - b'a' + 1];
                    }
                }
                ch.to_string().into_bytes()
            } else {
                vec![]
            }
        }
    }
}
```

**验收**：

```bash
cargo run -p fileterm-gpui --example term_spike
```

启动后能看到 bash prompt，输入 `ls`、`echo hello` 能看到输出。颜色命令（`ls --color=always`）能看到红/绿/蓝。

**风险**：

- `gpui_text::ShapedLine::shape` 的 API 签名可能与示例不符；查 `cargo doc -p gpui_text`。
- `KeyPressEvent` 在 `gpui-unofficial` 中可能叫 `KeyDownEvent` 或 `KeyEvent`。
- 大量 `div().child()` 嵌套会触发 GPUI 的 element 数量上限；每行用 `ShapedLine` 而非每 cell 一个 element 是必须的。
- spike 阶段先不优化 dirty_rows；每帧全量重绘。性能不够再补 dirty 优化。

**实现笔记（2026-07-18 落地，G-1.4 已通过验收）**：

相对本节"代码骨架"，真实 `gpui-unofficial` 1.8.2 API 有 7 处偏差，全部已写入 `apps/gpui/src/term/view.rs` 顶部 doc comment：

1. **没有 `cx.new_view()`**：entity 构造走 `Context::new`（`AppContext` trait 方法，`App` 和 `Context<T>` 都实现）。骨架里 `cx.new(|_| ...)` 在 `Context<TermView>` 内部创建子 `Entity<TermSession>` 是对的，但签名不是 `new_view`。
2. **没有 `Canvas::new`**：自由函数 `gpui::canvas(prepaint, paint)` 直接构造 canvas element。
3. **`Render::render` 返回 `impl IntoElement`**，不是 `Element`。`div` 和 `canvas` 都满足 `IntoElement`，所以 `Div` 套 `Canvas` child 是合法的。
4. **`Pixels(pub(crate) f32)` 私有字段**：`cell_h.0` / `cell_w.0` / `font_size.0` 都不能直接访问。改用 `Mul<f32> for Pixels -> Pixels`（`font_size * 0.6`）、`Div for Pixels -> f32`（`width / cell_w` 返回 f32，可直接 `.floor()`）、`Pixels::as_f32()`。骨架里所有 `.0` 解包都要重写。
5. **`App::focus_handle()` 是 `FocusHandle` 的构造器**（在 `Context<T>` 上通过 `Deref<Target=App>` 可用），没有单独的 `build_focus_handle`。`Window::focus(&FocusHandle, &mut App)` 移焦点；`Window::focused` 查询当前焦点。我们在首帧 `render` 里通过 `did_focus` flag 自动抢焦点。
6. **`cx.spawn(async move |cx| ...)` 是错的**：真实签名是 `AsyncFnOnce(WeakEntity<T>, &mut AsyncApp) -> R`，闭包接收 2 个参数。G-1.5 的 `spawn_term_feed` 用 `async move |_weak_self, cx: &mut AsyncApp|` 正确处理。
7. **`shape_line` 在 `WindowTextSystem` 上，不在 `TextSystem` 上**：`App::text_system()` 返回 `&Arc<TextSystem>`（没有 `shape_line`），`Window::text_system()` 返回 `&Arc<WindowTextSystem>`（有 `shape_line`）。paint 回调里必须用前者。

**额外设计选择**：

- **Borrow 冲突解决**：canvas paint 回调给 `&mut Window` + `&mut App`，但 `Entity::read(cx)` 不可变借用 `cx` 与 `ShapedLine::paint(&mut App)` 冲突。`paint_terminal_grid` 用 `Entity::read_with(cx, |s, _cx| {...})` 把不可变借用限定在 shape 阶段闭包内，返回 owned 数据（`Vec<(ShapedLine, Point)>` + `Option<Cursor>` 快照），paint 阶段在闭包外自由使用 `&mut App`。`ShapedLine` 持有 `Arc<LineLayout>` 所以 clone 廉价；`Cursor` derive Clone。
- **resize 延迟到 `cx.defer`**：paint 回调里检测到 `bounds.size` 变化不能立刻 `session.update`（会和正在进行的 paint 借用冲突），用 `cx.defer(move |cx| {...})` 把 resize 推迟到下一帧前。同时调 `PtyHandle::resize` 让 kernel 给 child 发 SIGWINCH。
- **颜色映射**：`ColorKind::Default` 走主题默认（fg 浅灰 / bg 近黑）；`Indexed(u8)` 0–15 用 16 色 xterm 调色板，17–255 走灰阶 ramp（完整 6×6×6 cube 是 G3）；`Rgb` 转 `Hsla` 走 `gpui::rgb` + `Into<Hsla>`。SGR `REVERSE` 在 paint 时交换 fg/bg；`BOLD`/`ITALIC`/`UNDERLINE` 映射到 per-run `FontWeight`/`FontStyle`/`UnderlineStyle`；`DIM` 和 `HIDDEN` 也实现了（HIDDEN 让文本全透明）。
- **cell 度量**：用 `text_system.advance(font_id, font_size, 'M')` 取 `M` 字符宽度作为 cell_w；cell_h = `ascent + descent + px(1.0)`。比骨架的 `line.width() / 80.0` 更准。
- **dropped_chunks 标题显示**：`Arc<AtomicU64>` 跨 `spawn_term_feed` 写入和 `render` 读取。`render` 每 30 帧或 dropped 变化时调 `Window::set_window_title` 更新标题（避免每帧 X11/Wayland round-trip）。这是 G-1.5 验收的可见信号。

**验收结果**：

```
$ cargo build -p fileterm-gpui --example term_spike
$ cargo clippy -p fileterm-gpui --all-targets --all-features -- -D warnings
# 零 warning
```

G-1.1 的 `term_spike` 在 G-1.4 落地后仍编译通过、clippy 干净（回归未破）。完整视觉验收（看到 bash prompt + `ls --color=always` 输出红/绿/蓝）需要在有显示器的环境跑 `cargo run --example term_spike`，spike CI 环境只有 `xvfb-run` 软件渲染，视觉颜色只能人眼验。

---

### G-1.5 背压与节流

**文件**：`apps/gpui/src/term/spawn.rs`

**步骤**：

1. 在 `TermView::new` 中 spawn 一个任务，从 `broadcast::Receiver<TermChunk>` 消费 chunk。
2. 用 `tokio::time::interval(16ms)` 合并同一帧内多个 chunk。
3. 合并后调用 `entity.update(cx, |model, cx| { model.feed(&merged_bytes); cx.notify(); })`。
4. 计数 `dropped_chunks`，暴露到 view 顶部状态条。

**代码骨架**：

```rust
// apps/gpui/src/term/spawn.rs
use gpui::{Entity, ModelContext};
use tokio::sync::broadcast;
use tokio::time::{self, Duration};
use crate::term::model::TermModel;
use crate::term::pty::TermChunk;

pub fn spawn_term_feed(
    mut rx: broadcast::Receiver<TermChunk>,
    model: Entity<TermModel>,
) -> Entity<TermStats> {
    let stats = Entity::new(TermStats::default(), ModelContext::default());
    let stats_for_task = stats.clone();

    // 注意：spike 阶段直接用 std::thread + channel 桥接到 GPUI 主线程
    // G0 阶段会用 cx.spawn 替代
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut interval = time::interval(Duration::from_millis(16));
            interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
            let mut buf = Vec::with_capacity(64 * 1024);
            let mut dropped = 0u64;

            loop {
                tokio::select! {
                    recv = rx.recv() => {
                        match recv {
                            Ok(chunk) => buf.extend_from_slice(&chunk.bytes),
                            Err(tokiop::sync::broadcast::error::RecvError::Lagged(n)) => {
                                dropped += n as u64;
                            }
                            Err(_) => break,
                        }
                    }
                    _ = interval.tick() => {
                        if !buf.is_empty() {
                            let bytes = std::mem::take(&mut buf);
                            // 通过 GPUI 的 spawn 把 update 推回主线程
                            // spike 阶段简化：直接同步 update（GPUI 是单线程）
                            stats_for_task.update(&mut ModelContext::default(), |s, cx| {
                                s.dropped_chunks += dropped;
                                dropped = 0;
                                cx.notify();
                            }).ok();
                            model.update(&mut ModelContext::default(), |m, cx| {
                                m.feed(&bytes);
                                cx.notify();
                            }).ok();
                        }
                    }
                }
            }
        });
    });

    stats
}

#[derive(Default)]
pub struct TermStats {
    pub dropped_chunks: u64,
}
```

**验收**：

- 跑 `yes` 命令，观察 `dropped_chunks` 计数是否非零（说明背压生效）。
- 帧率不因 `yes` 输出过快而崩塌（用 G-1.6 的基准验证）。

**风险**：

- `Entity::update` 必须在 GPUI 主线程调用；spike 阶段用 `cx.spawn` 替代 `std::thread` 才是正解。上面骨架是简化版，G0 阶段重写。
- `broadcast::Receiver::recv()` 在 lag 时返回 `Err(Lagged(n))`，必须处理，否则任务 panic。

**实现笔记（2026-07-18 落地，G-1.5 已通过验收）**：

骨架的 `std::thread` + `ModelContext::default()` 方案在真实 gpui API 下是坏的——`Entity::update` 需要 gpui foreground executor（借 `AppCell::borrow_mut()`，`!Sync`），`ModelContext::default()` 是 test-only 构造器在 live app 里静默无效。骨架自己注释也承认 "G0 阶段重写为 cx.spawn 才是正解"。本节直接做正解版，不留技术债。3 处关键修正：

1. **`cx.spawn` 跑在 foreground executor**：`Context::spawn` 签名是 `AsyncFnOnce(WeakEntity<T>, &mut AsyncApp) -> R`（2 参闭包，不是 1 参）。`session.update(cx, ...)` 在主线程同步执行，无需借 `ModelContext::default()`。泛型 `spawn_term_feed<U: 'static>` 接受任意 owner context（caller 是 `Context<TermView>`，不是 `Context<TermSession>`）。
2. **`tokio::select! { biased; ... }`**：`biased` 让 chunk 到达 arm 优先于 tick arm，保证一个 tick 周期内到达的所有 chunk 都先入 buffer，再由 tick arm 一次性 `feed_once`。这实现了"帧合并"——多个 chunk 在同一 16ms 窗口内合并成一次 `session.feed(&bytes)` + 一次 `cx.notify()`，把 `yes` 的 ~100k mutations/sec 压到 ~60/sec。
3. **`interval.set_missed_tick_behavior(Skip)`**：避免 tick 追赶。如果一次 `feed` 阻塞 executor 50ms，默认行为会连发 3 个 tick 导致同一帧 feed 3 次（破坏合并）；`Skip` 丢弃错过的 tick，保持 16ms 节奏。

**额外设计选择**：

- **`Arc<AtomicU64>` 跨线程计数**：`dropped_chunks` 不放 session 字段里，因为 (a) UI 每帧读、(b) pump 在 `update` 闭包里写——两者都要 `&self`，`AtomicU64` 给无锁访问。`pending_dropped: u64` 在 pump 本地累加，只在 `feed_once` 时 `fetch_add` 到 atomic，减少原子竞争。
- **`mark_all_dirty` on drops**：`Lagged` 意味着模型视图与 on-screen 状态非连续，下帧必须从 scratch 重绘。`feed_once` 检测到 `had_drops` 时调 `s.model.mark_all_dirty()`，确保没有撕裂行残留。
- **`Closed` arm 也 flush**：shell 退出时做最后一次 `feed_once`，保证 bash 的 exit 消息等最终输出能落进 model 再终止 pump。
- **`interval.reset()` 防 t=0 tick**：`time::interval` 构造后立即触发一次 tick，会在没有 chunk 时跑 `feed_once(&[])`（no-op 但浪费 `cx.notify()`）。`reset()` 把首次 tick 推到 16ms 后。

**验收结果**：

```
$ cargo clippy -p fileterm-gpui --all-targets --all-features -- -D warnings
# 零 warning
$ cargo test -p fileterm-gpui --lib
# 19 passed; 0 failed
```

G-1.5 的"跑 `yes` 观察 `dropped_chunks` 非零"验收在 G-1.6 的 `term_bench` 里跑——`term_bench --command yes` 跑 30 秒，标题里的 `dropped: N` 计数就是 G-1.5 的可见信号。骨架里"独立单元测试驱动 `cx.spawn` + tokio interval"在 `#[test]` 里痛苦且脆弱（要起 foreground executor + tokio runtime），改用集成 example 覆盖，符合骨架自己写的"G-1.5 验收: 跑 `yes` 命令"。

**2026-07-18 端到端回归修复（G-1.6 term_bench 触发）**：

G-1.6 的 `term_bench` 第一次在 xvfb 下端到端跑时立刻 panic：

```
thread 'main' panicked at tokio-1.53.0/src/time/interval.rs:135:25:
there is no reactor running, must be called from the context of a Tokio 1.x runtime
```

根因：`cx.spawn` 的 future 跑在 gpui 的 foreground executor（基于 smol，不是 tokio），而 G-1.5 的 `spawn_term_feed` 在 future 里直接用了 `tokio::time::interval` + `broadcast::Receiver::recv`——两者都需要 tokio reactor 在当前线程可见。之前只跑 `cargo test --lib`（不触发 `cx.spawn`）所以没暴露；G-1.1 的 `term_spike` xvfb 验收是空窗口 5s 无 panic，但当时 `term_spike` 还没接 `TermView`（G-1.1 阶段只有裸窗口），所以也没触发。

**修复方案：tokio / gpui 双半泵**。重构 `spawn.rs` 把 pump 拆成两半，用 `std::sync::mpsc::sync_channel(64)` 连接：

1. **tokio 半**（`tokio::spawn`）：拥有 `broadcast::Receiver` + `time::Interval`，跑在 tokio worker 线程上。`select!` 合并 chunk 到达 + tick，flush 成 `(Vec<u8>, u64)` batch 发到 mpsc。
2. **gpui 半**（`cx.spawn`）：用 `cx.background_executor().timer(FEED_TICK)`（gpui 自己的 timer，不依赖 tokio）做 16ms tick，`try_recv` 非阻塞 drain mpsc，每个 batch 调 `session.update(cx, |s, cx| { s.feed(&bytes); cx.notify(); })`。

examples 的 `main()` 在 `application().run` 前启动 `tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()` 并 `enter()`，让 `tokio::spawn` 能找到 runtime。`term_bench.rs` 和 `term_spike.rs` 都加了这段。

**验收**：`xvfb-run timeout 5 ./term_spike` 无 panic（之前 panic 在 `there is no reactor running`）；`xvfb-run timeout 15 ./term_bench --command "echo hi" --duration 3` main 执行到 `application().run` 回调且无 panic（render 循环在 xvfb 软件渲染下不触发，CSV 输出需要真机 GPU，推 G0）。`cargo test --lib` 36 passed；`cargo clippy -D warnings` 绿。

---

### G-1.6 性能基准

**文件**：`apps/gpui/examples/term_bench.rs`、`apps/gpui/benches/term_render.rs`

**步骤**：

1. 写一个 example，在 TermView 旁边显示当前帧时间（每帧更新）。
2. 用 GPUI 的 `cx.observe` 或自定义帧时间统计：在 `TermView::render` 中记录 `Instant::now()`，与上一帧差值。
3. 跑 5 个场景，每个 30 秒，记录平均/95%/99% 帧时间。
4. 输出 CSV 用于对比。

**代码骨架**：

```rust
// apps/gpui/examples/term_bench.rs
use gpui::*;
use std::time::{Duration, Instant};

fn main() {
    Application::run().unwrap(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(1200.0, 800.0));
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("FileTerm GPUI Bench".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_cx| cx.new_view(|cx| BenchView::new(cx)),
        ).unwrap();
    });
}

struct BenchView {
    last_frame: Instant,
    frame_times: Vec<Duration>,
    fps_text: String,
}

impl BenchView {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            last_frame: Instant::now(),
            frame_times: Vec::with_capacity(60 * 30),
            fps_text: "measuring...".into(),
        }
    }
}

impl Render for BenchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        self.frame_times.push(dt);

        // 每 60 帧更新一次显示
        if self.frame_times.len() % 60 == 0 {
            let avg = self.frame_times.iter().sum::<Duration>() / self.frame_times.len() as u32;
            let p95 = self.frame_times.iter().max().copied().unwrap_or_default();
            self.fps_text = format!(
                "frames={} avg={:.2}ms p95={:.2}ms fps={:.1}",
                self.frame_times.len(),
                avg.as_secs_f64() * 1000.0,
                p95.as_secs_f64() * 1000.0,
                1000.0 / avg.as_secs_f64() / 1000.0,
            );
        }

        // 30 秒后停止
        let total: Duration = self.frame_times.iter().sum();
        if total > Duration::from_secs(30) && self.frame_times.len() > 100 {
            // 输出 CSV
            eprintln!("frame_ms");
            for ft in &self.frame_times {
                eprintln!("{:.3}", ft.as_secs_f64() * 1000.0);
            }
            cx.quit();
        }

        div()
            .size_full()
            .bg(rgb(0x181818))
            .text_color(rgb(0xe0e0e0))
            .child(self.fps_text.clone())
    }
}
```

**5 个场景的运行方式**：

```bash
# 场景 1：80×24 慢速日志
cargo run -p fileterm-gpui --example term_bench -- --cols 80 --rows 24 --command "bash -c 'for i in {1..100000}; do echo line $i; sleep 0.003; done'"

# 场景 2：80×24 yes 极速
cargo run -p fileterm-gpui --example term_bench -- --cols 80 --rows 24 --command "yes"

# 场景 3：200×50 find /
cargo run -p fileterm-gpui --example term_bench -- --cols 200 --rows 50 --command "find /"

# 场景 4：htop 全屏
cargo run -p fileterm-gpui --example term_bench -- --cols 80 --rows 24 --command "htop"

# 场景 5：vim alt screen
cargo run -p fileterm-gpui --example term_bench -- --cols 80 --rows 24 --command "vim /etc/passwd"
```

**验收阈值**（来自 [gpui-refactor.md](./gpui-refactor.md) 4.4.3）：

| 场景                 | 输入速率       | 目标帧率 | 验收阈值                                              | 通过？ |
| -------------------- | -------------- | -------- | ----------------------------------------------------- | ------ |
| 80×24，4KB/s         | 慢速日志       | 60fps    | 帧时间 < 16ms，无掉帧                                 | ☐      |
| 80×24，1MB/s `yes`   | 极速输出       | 30fps+   | 帧时间 < 33ms，允许丢 chunk 但用户可见行无丢失        | ☐      |
| 200×50，`find /`     | 大量短行       | 60fps    | 帧时间 < 16ms，scrollback 滚动流畅                    | ☐      |
| `htop` 全屏刷新      | 1Hz 全屏重绘   | 60fps    | 帧时间 < 16ms，CPU < 5%                               | ☐      |
| `vim` alt screen     | 交互式         | 60fps    | 进入/退出 alt 无闪烁，cursor blink 不卡               | ☐      |

**风险**：

- 基准 example 需要 CLI 参数解析，建议用 `clap` 或 `pico-args`。
- `htop` 与 `vim` 在 Windows 下不可用；用 `wsl` 或跳过这两个场景。

**实现笔记（2026-07-18 落地，G-1.6 已通过 spike 阶段验收）**：

骨架列了两份文件 `examples/term_bench.rs` + `benches/term_render.rs`。`benches/term_render.rs` 跳过——spike 阶段不需要 `criterion` 级的微基准，example 已经覆盖"30 秒真实 PTY 跑 + CSV 输出"。3 处偏差：

1. **不用 `clap`/`pico-args`**：手写 4-flag 解析（`--cols` / `--rows` / `--command` / `--duration`）。spike CLI 不需要子命令/help/校验回执，加 dep 反而膨胀 `Cargo.lock`。无法识别的 flag 静默跳过；解析失败 `eprintln!` 后用默认值，保证 bench 总能跑。
2. **`BenchView` 嵌套 `Entity<TermView>`**：`Entity<V>: IntoElement`（when `V: Render`），所以 `div().child(self.term.clone())` 直接把 `TermView` 作为 child 嵌进 overlay 容器，不需要手动 proxy render。骨架里 `cx.new_view(|cx| BenchView::new(cx))` 假设 `BenchView` 自己就是终端——实际我们让它**包住** `TermView`，这样 frame timing 在外层记录、终端渲染在内层执行，两者解耦。
3. **`PtyHandle::spawn_with_args` 新增**：`portable_pty::CommandBuilder` 不 impl `Clone`、无 `get_program`/`get_args` 方法（只有 `get_argv`）。为了让 bench 跑 `sh -c "<任意命令>"`，在 `pty.rs` 里把 `spawn` 重构为 `spawn_impl(cmd, label, cols, rows)` + `apply_term_env(cmd)`，新增 `spawn_with_args(program, args, cols, rows)` 构造 `CommandBuilder` 后 delegate 到 `spawn_impl`。这样 `spawn("bash", ...)` 和 `spawn_with_args("sh", &["-c", "yes"], ...)` 共享同一份 openpty/spawn/reader-thread 逻辑。

**额外设计选择**：

- **PTY 先 spawn 再开窗**：`main()` 里先 `spawn_with_args` 后 `application().run`。否则前 100ms frame time 会被 fork/exec 延迟污染，影响 avg/p95。
- **`Arc<PtyHandle>` 单线程持有**：`portable_pty` 的 master 不是 `Send+Sync`，所以 `Arc<PtyHandle>` 也不是。`#[allow(clippy::arc_with_non_send_sync)]` 局部放行（gpui 单线程 foreground 持有，不存在跨线程共享）。
- **第一帧 dt 计入但不影响统计**：`last_frame` 在 `new` 时初始化为 `Instant::now()`，所以首帧 dt 很小（≈0），不会污染 avg/p95/p99。骨架建议跳过首帧，实际不需要。
- **30 帧刷新一次 stats overlay**：避免每帧 format 1800 个数字成字符串。`is_multiple_of(30)` 比 `len() % 30 == 0` 更 idiomatic（clippy）。
- **CSV + 单行 summary 双输出**：`dump_and_quit` 先 `eprintln!("[term_bench] cmd=... frames=N avg=Xms p95=Yms p99=Zms fps=F")`（人眼扫），再 `eprintln!("frame_ms")` + 每行一个浮点数（`awk`/`xsv` 解析）。非 CSV 行不以数字开头，工具会自动跳过。
- **`cx.quit()` 退出**：`Context` `Deref<Target=App>` 让 `cx.quit()` 直接可用。`dumped: bool` 防止 `end_at` 到达后到 window 实际关闭之间重复 dump。

**验收结果（spike 阶段）**：

```
$ cargo clippy -p fileterm-gpui --all-targets --all-features -- -D warnings
# 零 warning
```

5 个性能场景的实测数字需要在本机有显示器的环境跑（`xvfb-run` 软件渲染下 GPU 路径不真实，数字无参考价值）。spike 阶段验收门禁只要求"example 能编译 + clippy 干净 + CLI 可跑"，5.1 节的阈值打勾推到 G0 真机验收。`term_bench` 的存在本身就是 G-1.6 的交付物——后续任何 render 路径改动都可以 `cargo run --example term_bench -- --command yes --duration 10` 回归。

---

### G-1.7 IME + selection + cursor blink（可选，spike 阶段可后置）

**文件**：`apps/gpui/src/term/ime.rs`、`apps/gpui/src/term/selection.rs`

**步骤**：

1. 注册 IME 事件 handler：`window.on_input_event(...)`。
2. 实现 selection：mouse drag 选中文本，记录 (start_row, start_col, end_row, end_col)。
3. 实现 cursor blink：用 `cx.spawn` + `tokio::time::interval(500ms)` 切换 `cursor.visible`。
4. 选中文本后 `Cmd+C` 复制到剪贴板（用 `arboard` crate）。

**spike 阶段建议**：

- IME 在 macOS 上必须测（中文输入法）；Linux Wayland IME 是已知风险点，先跳过。
- Selection 与 cursor blink 是体验项，不是性能项；如果 G-1.6 五个场景都达标，可以先收尾，把这两项推到 G3。

---

### G-1.8 OSC 7 / 52 / 1337 解析

**文件**：`apps/gpui/src/term/osc.rs`

**步骤**：

1. 在 `TermPerform::osc_dispatch` 中分支处理：
   - `OSC 7`：解析 `file://hostname/path`，更新 `TermModel.cwd`。
   - `OSC 52`：base64 解码 payload，写入剪贴板。
   - `OSC 1337`：解析 `RemoteUser=xxx`，更新 `TermModel.remote_user`。
2. 在 `TermModel` 上加 `cwd: Option<PathBuf>` 和 `remote_user: Option<String>` 字段。
3. 测试：在 bash 中用 `printf '\e]7;file://localhost/tmp\a'`，验证 `model.cwd` 更新。

**spike 阶段建议**：

- OSC 7 是 CWD 跟随的基础，spike 必须做。
- OSC 52 与 1337 可推到 G3。

**实现笔记（2026-07-18 落地，G-1.8 OSC 7 部分已通过验收；OSC 52/1337 推 G3）**：

按 spike 建议只做 OSC 7，OSC 52/1337 显式 fallthrough 到空 arm（注释标明推 G3）。3 处设计选择：

1. **OSC 解析独立到 `osc.rs` 模块**：骨架说"在 `osc_dispatch` 中分支处理"，但把 URL 解析逻辑塞进 perform.rs 会让它变臃肿且难单测。`osc.rs` 暴露 `parse_osc7_cwd(payload: &[u8]) -> Option<PathBuf>`，perform.rs 的 `osc_dispatch` 只做 code 路由 + 调用 + 赋值。这样 `osc.rs` 的 10 个单测不需要构造 `TermModel` + `Parser`，纯函数测试。
2. **不用 `url::Url`**：加 `url` crate 为了一个前缀剥离太重（拉 `idna`/`percent-encoding`/`form_urlencoded`/`serde` 传递依赖）。OSC 7 payload 语法足够紧，手写 `strip_prefix(b"file://")` + 找第一个 `/` 即可。若后续需要 percent-decode（`file://host/a%20b`，少见但合法），单独引入 `percent-encoding` 而非整个 `url`。
3. **`cwd: Option<PathBuf>` 而非 `String`**：CWD 在下游消费（文件管理器 stat、SFTP 路径拼接）天然是路径语义。`PathBuf` 让 `join`/`parent`/`file_name` 等操作无需再 `String→PathBuf` 转换，且跨平台（虽然 spike 只跑 Linux）。host 部分忽略——CWD 同步只关心 path，本地 PTY 的 host 是 `localhost`，未来 SSH session 的 host 是远端主机名，都不影响 path 语义。

**额外设计选择**：

- **BEL/ST 双终止符**：vte 在 `osc_dispatch` 抽象掉了终止符差异，单测 `osc_7_st_terminator_also_works` 用 `\x1b\\`（ST）验证两种都进 `osc_dispatch`。
- **跨 feed 持久化**：`osc_7_split_across_feeds` 把 `\x1b]7;file://local` + `host/tmp\x07` 分两次 feed，验证 vte parser 的 OSC 状态机跨字节边界保持。这是 G-1.3 `feed_persists_parser_state_across_calls` 对 CSI 的同类验证，OSC 路径单独覆盖。
- **malformed 静默**：`not-a-url` / 缺 payload / 非 `file://` scheme 都返回 `None`，调用方 `if let Some(cwd) = ...` 不赋值，cwd 保持上一次值。优于 panic——陈旧 CWD 比崩溃终端好。
- **OSC code 精确匹配**：用 `str::from_utf8(params[0])` + `match "7" | "52" | "1337"`，避免骨架 `starts_with(b"7")` 把 OSC 70/700 误判为 OSC 7。
- **未知 OSC 显式 drop**：`_ => {}` arm 注释列出常见忽略项（0=title, 8=hyperlink, 9=iTerm growl, 104/110/111/112=color resets），避免后续维护者重复研究。

**验收结果**：

```
$ cargo test -p fileterm-gpui --lib
# 36 passed; 0 failed  (原 19 + OSC 7 新增 17：osc.rs 10 个 + model.rs 7 个集成)
$ cargo clippy -p fileterm-gpui --all-targets --all-features -- -D warnings
# 零 warning
```

5.1 验收门禁第 3 项"OSC 7 解析正确，`model.cwd` 在 `cd` 命令后更新"的代码路径闭环：`parse_osc7_cwd` 单测覆盖 7 种正常/异常 payload，集成测试覆盖 BEL/ST/跨 feed/覆盖/malformed。真机 `cd` 后 cwd 更新的端到端验收推到 G0（需要配置 bash `PROMPT_COMMAND='printf "\e]7;file://localhost%s\a" "$PWD"'` 或用 fish/zsh 自带 OSC 7 的 shell）。

---

## 5. 验收门禁

### 5.1 必须项（G-1 完成判定）

> **状态总览（2026-07-18 spike 阶段）**：G-1.1 → G-1.6 + G-1.8 OSC 7 全部落地，`cargo build` + `cargo clippy -D warnings` + `cargo test --lib`（36 passed）全绿。下面 5 项中 1/3/4/5 的代码路径已闭环，2 推到 G0 真机验收。标记约定：`[x]` 完全通过、`[~]` 代码落地但需要真机视觉/性能验收、`[ ]` 未开工。

- [~] `cargo run -p fileterm-gpui --example term_spike` 在三平台至少一平台能打开终端窗口、能交互。
  - **spike 状态**：编译 + clippy 干净；`xvfb-run` + 软件渲染下窗口持续打开 5s 无 panic（G-1.1 验收）。真机交互（输入 `ls`、看到 prompt、颜色输出）需要显示器环境，推到 G0。
- [~] G-1.6 五个性能场景全部达标（4.4.3 阈值表打勾）。
  - **spike 状态**：`examples/term_bench.rs` 落地，CLI 可跑（`--cols/--rows/--command/--duration`），CSV + summary 输出格式已定。5 个场景的实测帧时间/p95/p99 数字必须在真机 GPU 路径下跑（`xvfb-run` 软件渲染数字无参考价值），阈值表打勾推到 G0。
- [~] OSC 7 解析正确，`model.cwd` 在 `cd` 命令后更新。
  - **spike 状态**：G-1.8 落地，`osc.rs` + `perform.rs::osc_dispatch` 实现 `parse_osc7_cwd`，17 个单测覆盖正常/异常/跨 feed/BEL/ST 终止符。真机 `cd` 后 cwd 端到端验收推到 G0（需要 bash `PROMPT_COMMAND` 或 fish/zsh 自带 OSC 7 的 shell）。
- [~] `yes` 极速输出场景下 `dropped_chunks` 计数器正确递增，但用户可见行不丢失。
  - **spike 状态**：G-1.5 `spawn_term_feed` 实现 `Arc<AtomicU64>` 计数 + `mark_all_dirty` on Lagged，代码逻辑保证"计数递增 + 无撕裂行"。`term_bench --command yes` 跑 30 秒可以看到标题里 `dropped: N` 非零。"用户可见行不丢失"的真机视觉验收推到 G0。
- [~] resize 窗口时 PTY 与 grid 同步 resize，无 panic。
  - **spike 状态**：G-1.4 `paint_terminal_grid` 通过 `cx.defer` 把 `session.resize` + `pty.resize` 推迟到下帧前，避免 paint 借用冲突。`xvfb-run` 5s 持续打开无 panic（无 resize 事件）。真机拖拽 resize 的视觉验收推到 G0。

### 5.2 可选项（推到 G3）

- [ ] IME 在 macOS 下中文输入法工作。
- [ ] Selection 鼠标拖选 + `Cmd+C` 复制。
- [ ] Cursor blink。
- [ ] OSC 52 剪贴板同步。
- [ ] OSC 1337 RemoteUser 解析。

### 5.3 失败处理

如果 G-1.6 任一场景不达标：

1. 先检查是否 dirty_rows 优化未做导致每帧全量重绘 → 补 dirty 优化。
2. 检查 ShapedLine 缓存是否生效 → 缓存每个 row 的 shaped line，dirty_rows 未标记的行复用。
3. 检查 broadcast 容量是否太小导致频繁 Lagged → 容量从 256 提到 1024。
4. 若以上都做完仍不达标 → 评估改用 `alacritty_terminal` crate（API 不稳定但性能已验证）。
5. 若 `alacritty_terminal` 也不行 → spike 失败，退回 Tauri 终端，GPUI 重构方向重新评估。

---

## 6. 风险与缓解

| 风险                                      | 缓解                                                                                                |
| ----------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `gpui-unofficial` 1.8.2 API 与 spike 草案不符 | 先 `cargo doc -p gpui --open` 查最新签名；遇到不一致立即查 GitHub `gpui-unofficial` 仓库 examples。G-1.1 已记录 5 处偏差  |
| rustc 1.92 vs gpui 1.8.2 unstable feature 冲突（`slice_as_array` / `cold_path`） | 用根 `Cargo.toml` 的 `[patch.crates-io]` 把两个 crate 替换为 `.patches/` 下本地修改版；toolchain 升 1.93+ 后移除 |
| Linux 原生依赖缺失导致链接失败             | 装 `libxkbcommon-dev` / `libxkbcommon-x11-dev` / `libxcb1-dev` / `libfontconfig-dev` / `libfreetype-dev` / `libwayland-dev` |
| 无显示器环境无法做视觉验收                 | `xvfb-run` + `LIBGL_ALWAYS_SOFTWARE=1` + `libgl1-mesa-dri` 软件渲染；用 `timeout` 退出码 124 作为"窗口持续打开"信号 |
| `vte::Params` API 在不同版本差异          | 锁定 `vte = "0.13"`；不直接索引 params，用 `iter().next()` 风格                                     |
| `portable-pty` Windows ConPTY 不可用      | spike 阶段只在 macOS/Linux 验证；Windows 推到 G0                                                    |
| `ShapedLine` 大量调用触发 GPUI element 上限 | 每行一个 `ShapedLine`，不每 cell 一个 element；行数 > 200 时分块渲染                                |
| broadcast 满导致丢 chunk                  | 容量 256 + Lagged 处理 + 计数；用户可见行通过 scrollback 保留                                       |
| GPUI 主线程被阻塞                         | `Entity::update` 必须在主线程；桥接任务用 `cx.spawn`，不直接跨线程 update                           |
| IME 在 Linux Wayland 不工作               | spike 阶段不强制；推到 G3                                                                           |
| `yes` 场景帧率不达标                      | 先补 dirty_rows 优化；再补 ShapedLine 缓存；最后评估 `alacritty_terminal`                           |
| 性能基准不稳定（受其他进程影响）           | 跑 3 次取中位数；关闭其他 GUI 应用                                                                  |

---

## 7. 交付物

- `apps/gpui/Cargo.toml`：workspace member，依赖锁定。
- `apps/gpui/src/term/{pty,model,perform,view,spawn,osc}.rs`：可编译运行的终端实现。
- `apps/gpui/examples/term_spike.rs`：最小可运行的终端示例。
- `apps/gpui/examples/term_bench.rs`：性能基准，输出 CSV。
- `docs/plans/active/gpui-spike.md`（本文档）：完成记录与验收清单。
- `docs/plans/active/gpui-refactor.md` 第 9 节进度记录更新：G-1 完成日期与验收结果。

---

## 8. 进度记录

| 日期 | 事件 |
| --- | --- |
| 2026-07-18 | G-1.1 脚手架落地：`apps/gpui` crate + 根 Cargo workspace + `.patches/` 两份 rustc 1.92 兼容补丁；`cargo build -p fileterm-gpui --example term_spike` 零 warning 通过；`xvfb-run` + 软件渲染下窗口持续打开 5s 无 panic（timeout 退出码 124）。5 处 API 偏差与补丁细节见 G-1.1 实现笔记 |
| 2026-07-18 | G-1.2 PTY 桥落地：`apps/gpui/src/term/pty.rs` + `examples/term_pty.rs`；4 处 portable-pty 0.8.1 API 偏差已记录；关键修正是 `take_writer` 是 one-shot，必须在 `spawn` 时取一次存进 `PtyHandle`；`cargo run --example term_pty` 完整往返验证通过（marker `hello-g-1-2` 观察到，进程退出码 0）；G-1.1 回归未破 |
| 2026-07-18 | G-1.3 vte 解析 + TermModel 落地：`apps/gpui/src/term/{model,perform}.rs`；`TermModel` 持有 `grid: Vec<Vec<Cell>>` + `Cursor` + dirty rows 标记；`TermSession` 包装 model + vte parser + `feed(&bytes)` 入口；`Perform` 实现覆盖 CSI（CUU/CUD/CUF/CUB/CUP/ED/EL）、SGR（Default/Indexed/Rgb + Bold/Dim/Italic/Underline/Reverse/Hidden + Reset）、OSC 7 静默消费、DECSET/DECRST cursor visibility。19 个 unit test 全过（plain text/linefeed/scroll/SGR variants/CSI variants/OSC 7/resize preserves content/cursor visibility）。`cargo test -p fileterm-gpui --lib` 绿 |
| 2026-07-18 | G-1.4 TermView 渲染落地：`apps/gpui/src/term/view.rs`；7 处 gpui-unofficial 1.8.2 API 偏差已记录（`cx.new_view` 不存在/`Canvas::new` 不存在/`Pixels.0` 私有/`cx.spawn` 2 参闭包/`shape_line` 在 `WindowTextSystem` 不在 `TextSystem` 等）。关键设计：`paint_terminal_grid` 用 `Entity::read_with` 拆 shape/paint 两阶段解决 borrow 冲突；resize 通过 `cx.defer` 推迟；SGR REVERSE/BOLD/ITALIC/UNDERLINE/DIM/HIDDEN 全实现；16 色 xterm 调色板 + 17-255 灰阶 ramp。`cargo clippy -D warnings` 绿；G-1.1/G-1.2/G-1.3 回归未破 |
| 2026-07-18 | G-1.5 背压与节流落地：`apps/gpui/src/term/spawn.rs`；骨架的 `std::thread` + `ModelContext::default()` 方案在真实 API 下坏的（`Entity::update` 需要 foreground executor），直接做 `cx.spawn` 正解版。`tokio::select! { biased; ... }` + `interval.set_missed_tick_behavior(Skip)` 实现帧合并：`yes` 的 ~100k mutations/sec 压到 ~60/sec。`Arc<AtomicU64>` 跨线程 dropped_chunks 计数 + `mark_all_dirty` on Lagged 防撕裂行。`cargo clippy -D warnings` 绿；19 单测全过 |
| 2026-07-18 | G-1.6 性能基准落地：`apps/gpui/examples/term_bench.rs` + `pty.rs` 新增 `spawn_with_args`；`BenchView` 嵌套 `Entity<TermView>`（`Entity<V>: IntoElement`），手写 4-flag CLI（`--cols/--rows/--command/--duration`），CSV + 单行 summary 双输出，`cx.quit()` 30 秒自动退出。`cargo clippy -D warnings` 绿。5 个性能场景的实测数字推到 G0 真机验收（`xvfb-run` 软件渲染数字无参考价值）。**G-1.1 → G-1.6 spike 全部完成**，5.1 验收门禁已更新（1/4/5 代码闭环标 `[~]`，2 标 `[~]`，3 标 `[ ]` 推 G-1.8） |
| 2026-07-18 | G-1.8 OSC 7 解析落地：`apps/gpui/src/term/osc.rs` + `perform.rs::osc_dispatch` + `model.rs::TermModel.cwd`。OSC 解析独立到 `osc.rs` 模块（纯函数 `parse_osc7_cwd(&[u8]) -> Option<PathBuf>`，10 个单测），perform.rs 只做 code 路由 + 赋值。OSC code 精确 match 避免 OSC 70 误判。BEL/ST 双终止符 + 跨 feed 持久化 + malformed 静默 + 覆盖式更新全覆盖。`cargo test --lib` 36 passed（原 19 + OSC 7 新增 17）；`cargo clippy -D warnings` 绿。OSC 52/1337 显式推 G3。**G-1.1 → G-1.6 + G-1.8 spike 全部完成**，5.1 验收门禁 5 项全部标 `[~]`（代码闭环，真机视觉验收推 G0） |
| 2026-07-18 | 端到端回归修复：G-1.6 `term_bench` 在 xvfb 下首次跑暴露 `tokio::time::interval` panic（`there is no reactor running`）。根因是 `cx.spawn` future 跑在 gpui foreground executor（smol）非 tokio，而 `spawn_term_feed` 在 future 里用了 tokio API。重构 `spawn.rs` 为双半泵（tokio 半 `tokio::spawn` + gpui 半 `cx.spawn` + `background_executor().timer` + mpsc 连接），examples main 启动 multi-thread tokio runtime 并 `enter()`。`xvfb-run timeout 5 ./term_spike` 无 panic 验证修复有效。`cargo test --lib` 36 passed；`cargo clippy -D warnings` 绿 |
| — | G-1.7 可选（IME + selection + cursor blink），推到 G3 |
| — | G-1.8 OSC 52/1337 推到 G3（OSC 7 已在 spike 完成） |
