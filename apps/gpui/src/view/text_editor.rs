use std::ops::Range;

use gpui::{
    div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, Context,
    CursorStyle, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, IntoElement, KeyDownEvent, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, ShapedLine,
    SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use zeroize::Zeroize;

use crate::theme::ThemePalette;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextInputMode {
    SingleLine,
    MultiLine,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TextInputEvent {
    Changed(String),
    Submit,
    Save,
    Cancel,
}

pub(crate) struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    mode: TextInputMode,
    secret: bool,
    height: Pixels,
    headless: bool,
    auto_focus: bool,
    palette: ThemePalette,
}

impl Drop for TextInput {
    fn drop(&mut self) {
        if self.secret {
            self.content.zeroize();
        }
    }
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub(crate) fn new(
        content: impl Into<String>,
        placeholder: impl Into<SharedString>,
        mode: TextInputMode,
        secret: bool,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = content.into();
        let cursor = content.len();
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            content,
            placeholder: placeholder.into(),
            selected_range: cursor..cursor,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            mode,
            secret,
            height: match mode {
                TextInputMode::SingleLine => px(38.0),
                TextInputMode::MultiLine => px(76.0),
            },
            headless: false,
            auto_focus: false,
            palette,
        }
    }

    pub(crate) fn set_value(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        let value = value.into();
        if self.content == value {
            return;
        }
        if self.secret {
            self.content.zeroize();
        }
        self.content = value;
        let cursor = self.content.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.emit(TextInputEvent::Changed(self.content.clone()));
        cx.notify();
    }

    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_value(String::new(), cx);
    }

    pub(crate) fn value(&self) -> &str {
        &self.content
    }

    pub(crate) fn content_with_cursor(&self) -> String {
        with_visible_cursor(&self.content, self.cursor_offset())
    }

    pub(crate) fn set_secret(&mut self, secret: bool, cx: &mut Context<Self>) {
        if self.secret != secret {
            self.secret = secret;
            cx.notify();
        }
    }

    pub(crate) fn set_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        if self.height != height {
            self.height = height;
            cx.notify();
        }
    }

    pub(crate) fn set_headless(&mut self, headless: bool, cx: &mut Context<Self>) {
        if self.headless != headless {
            self.headless = headless;
            cx.notify();
        }
    }

    pub(crate) fn request_focus(&mut self, cx: &mut Context<Self>) {
        self.auto_focus = true;
        cx.notify();
    }

    pub(crate) fn set_palette(&mut self, palette: ThemePalette) {
        self.palette = palette;
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = valid_cursor(&self.content, offset);
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = valid_cursor(&self.content, offset);
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.marked_range = None;
        cx.notify();
    }

    fn replace_range(&mut self, range: Range<usize>, new_text: &str) {
        let start = valid_cursor(&self.content, range.start);
        let end = valid_cursor(&self.content, range.end.max(start));
        self.content.replace_range(start..end, new_text);
        let cursor = start + new_text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
    }

    fn emit_changed(&self, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Changed(self.content.clone()));
        cx.notify();
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        utf16_to_utf8_offset(&self.content, offset)
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        utf8_to_utf16_offset(&self.content, offset)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn display_text(&self) -> String {
        let content = if self.secret {
            "•".repeat(self.content.chars().count())
        } else if self.mode == TextInputMode::MultiLine {
            self.content.replace('\n', " ")
        } else {
            self.content.clone()
        };
        if content.is_empty() {
            self.placeholder.to_string()
        } else {
            content
        }
    }

    fn display_offset(&self, content_offset: usize) -> usize {
        let content_offset = valid_cursor(&self.content, content_offset);
        if self.secret {
            self.content[..content_offset].chars().count() * '•'.len_utf8()
        } else {
            content_offset
        }
    }

    fn content_offset(&self, display_offset: usize) -> usize {
        if !self.secret {
            return valid_cursor(&self.content, display_offset.min(self.content.len()));
        }
        let character_count = display_offset / '•'.len_utf8();
        self.content
            .char_indices()
            .map(|(offset, _)| offset)
            .nth(character_count)
            .unwrap_or(self.content.len())
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.x <= bounds.left() {
            return 0;
        }
        if position.x >= bounds.right() {
            return self.content.len();
        }
        self.content_offset(line.closest_index_for_x(position.x - bounds.left()))
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let command = event.keystroke.modifiers.platform || event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        if command && event.keystroke.key == "s" {
            cx.stop_propagation();
            cx.emit(TextInputEvent::Save);
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                cx.emit(TextInputEvent::Cancel);
            }
            "tab" => {
                cx.stop_propagation();
                if shift {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
            }
            "enter" | "return" if self.mode == TextInputMode::MultiLine && !command => {
                cx.stop_propagation();
                self.replace_text_in_range(None, "\n", window, cx);
            }
            "enter" | "return" => {
                cx.stop_propagation();
                cx.emit(TextInputEvent::Submit);
            }
            "left" => {
                cx.stop_propagation();
                let offset = previous_char_boundary(&self.content, self.cursor_offset());
                if shift {
                    self.select_to(offset, cx);
                } else if self.selected_range.is_empty() {
                    self.move_to(offset, cx);
                } else {
                    self.move_to(self.selected_range.start, cx);
                }
            }
            "right" => {
                cx.stop_propagation();
                let offset = next_char_boundary(&self.content, self.cursor_offset());
                if shift {
                    self.select_to(offset, cx);
                } else if self.selected_range.is_empty() {
                    self.move_to(offset, cx);
                } else {
                    self.move_to(self.selected_range.end, cx);
                }
            }
            "up" if self.mode == TextInputMode::MultiLine => {
                cx.stop_propagation();
                let offset = move_cursor_vertically(&self.content, self.cursor_offset(), -1);
                if shift {
                    self.select_to(offset, cx);
                } else {
                    self.move_to(offset, cx);
                }
            }
            "down" if self.mode == TextInputMode::MultiLine => {
                cx.stop_propagation();
                let offset = move_cursor_vertically(&self.content, self.cursor_offset(), 1);
                if shift {
                    self.select_to(offset, cx);
                } else {
                    self.move_to(offset, cx);
                }
            }
            "home" => {
                cx.stop_propagation();
                if shift {
                    self.select_to(line_start(&self.content, self.cursor_offset()), cx);
                } else {
                    self.move_to(line_start(&self.content, self.cursor_offset()), cx);
                }
            }
            "end" => {
                cx.stop_propagation();
                if shift {
                    self.select_to(line_end(&self.content, self.cursor_offset()), cx);
                } else {
                    self.move_to(line_end(&self.content, self.cursor_offset()), cx);
                }
            }
            "backspace" => {
                cx.stop_propagation();
                if self.selected_range.is_empty() {
                    let cursor = self.cursor_offset();
                    let previous = previous_char_boundary(&self.content, cursor);
                    if previous == cursor {
                        window.play_system_bell();
                        return;
                    }
                    self.selected_range = previous..cursor;
                }
                self.replace_text_in_range(None, "", window, cx);
            }
            "delete" => {
                cx.stop_propagation();
                if self.selected_range.is_empty() {
                    let cursor = self.cursor_offset();
                    let next = next_char_boundary(&self.content, cursor);
                    if next == cursor {
                        window.play_system_bell();
                        return;
                    }
                    self.selected_range = cursor..next;
                }
                self.replace_text_in_range(None, "", window, cx);
            }
            "a" if command => {
                cx.stop_propagation();
                self.selected_range = 0..self.content.len();
                self.selection_reversed = false;
                cx.notify();
            }
            "c" if command => {
                cx.stop_propagation();
                if !self.selected_range.is_empty() && !self.secret {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.content[self.selected_range.clone()].to_string(),
                    ));
                }
            }
            "x" if command => {
                cx.stop_propagation();
                if !self.selected_range.is_empty() && !self.secret {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.content[self.selected_range.clone()].to_string(),
                    ));
                    self.replace_text_in_range(None, "", window, cx);
                }
            }
            "v" if command => {
                cx.stop_propagation();
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    let text = if self.mode == TextInputMode::SingleLine {
                        text.replace(['\r', '\n'], " ")
                    } else {
                        text.replace("\r\n", "\n").replace('\r', "\n")
                    };
                    self.replace_text_in_range(None, &text, window, cx);
                }
            }
            _ => {}
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.focus_handle.focus(window, cx);
        self.is_selecting = true;
        let offset = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = false;
        cx.stop_propagation();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
            cx.stop_propagation();
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_text = if self.mode == TextInputMode::SingleLine {
            new_text.replace(['\r', '\n'], " ")
        } else {
            new_text.replace("\r\n", "\n").replace('\r', "\n")
        };
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        self.replace_range(range, &new_text);
        self.marked_range = None;
        self.emit_changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_text = if self.mode == TextInputMode::SingleLine {
            new_text.replace(['\r', '\n'], " ")
        } else {
            new_text.replace("\r\n", "\n").replace('\r', "\n")
        };
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let start = range.start;
        self.replace_range(range, &new_text);
        self.marked_range = (!new_text.is_empty()).then_some(start..start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range| {
                let relative = utf16_range_to_utf8(&new_text, range);
                start + relative.start..start + relative.end
            })
            .unwrap_or(start + new_text.len()..start + new_text.len());
        self.selection_reversed = false;
        self.emit_changed(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + layout.x_for_index(self.display_offset(range.start)),
                bounds.top(),
            ),
            point(
                bounds.left() + layout.x_for_index(self.display_offset(range.end)),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        let display_index = layout.index_for_x(point.x - bounds.left())?;
        Some(self.offset_to_utf16(self.content_offset(display_index)))
    }
}

struct TextInputElement {
    input: Entity<TextInput>,
}

struct TextInputPrepaint {
    line: ShapedLine,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextInputElement {
    type RequestLayoutState = ();
    type PrepaintState = TextInputPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content_empty = input.content.is_empty();
        let display_text: SharedString = input.display_text().into();
        let style = window.text_style();
        let base_run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: if content_empty {
                input.palette.text_soft
            } else {
                input.palette.text
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked_display_range = input
            .marked_range
            .as_ref()
            .map(|range| input.display_offset(range.start)..input.display_offset(range.end));
        let runs = if let Some(marked) = marked_display_range {
            vec![
                TextRun {
                    len: marked.start,
                    ..base_run.clone()
                },
                TextRun {
                    len: marked.end.saturating_sub(marked.start),
                    underline: Some(UnderlineStyle {
                        color: Some(input.palette.accent),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..base_run.clone()
                },
                TextRun {
                    len: display_text.len().saturating_sub(marked.end),
                    ..base_run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![base_run]
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);
        let cursor = input.display_offset(input.cursor_offset());
        let selected = input.display_offset(input.selected_range.start)
            ..input.display_offset(input.selected_range.end);
        let (selection, cursor) = if !content_empty && !selected.is_empty() {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(selected.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selected.end),
                            bounds.bottom(),
                        ),
                    ),
                    input.palette.accent_surface,
                )),
                None,
            )
        } else {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + line.x_for_index(cursor), bounds.top()),
                        size(px(1.5), bounds.size.height),
                    ),
                    input.palette.accent,
                )),
            )
        };
        TextInputPrepaint {
            line,
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus_handle, headless) = {
            let input = self.input.read(cx);
            (input.focus_handle.clone(), input.headless)
        };
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if !headless {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection);
            }
            prepaint
                .line
                .paint(
                    bounds.origin,
                    window.line_height(),
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .expect("paint text input");
            if focus_handle.is_focused(window) {
                if let Some(cursor) = prepaint.cursor.take() {
                    window.paint_quad(cursor);
                }
            }
        }
        self.input.update(cx, |input, _| {
            input.last_layout = Some(prepaint.line.clone());
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.auto_focus {
            self.auto_focus = false;
            self.focus_handle.focus(window, cx);
        }
        div()
            .key_context("TextInput")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .h(self.height)
            .w_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .cursor(CursorStyle::IBeam)
            .when(self.headless, |view| view.absolute().inset_0())
            .when(!self.headless, |view| {
                view.px_3()
                    .rounded_md()
                    .bg(self.palette.background)
                    .border_1()
                    .border_color(if self.focus_handle.is_focused(window) {
                        self.palette.accent
                    } else {
                        self.palette.border
                    })
                    .text_sm()
            })
            .child(TextInputElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn utf16_to_utf8_offset(content: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_offset = 0;
    for character in content.chars() {
        if utf16_offset >= offset {
            break;
        }
        utf16_offset += character.len_utf16();
        utf8_offset += character.len_utf8();
    }
    utf8_offset
}

fn utf8_to_utf16_offset(content: &str, offset: usize) -> usize {
    let offset = valid_cursor(content, offset);
    content[..offset].encode_utf16().count()
}

fn utf16_range_to_utf8(content: &str, range: &Range<usize>) -> Range<usize> {
    utf16_to_utf8_offset(content, range.start)..utf16_to_utf8_offset(content, range.end)
}

pub(crate) fn valid_cursor(content: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(content.len());
    while !content.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

pub(crate) fn previous_char_boundary(content: &str, cursor: usize) -> usize {
    let cursor = valid_cursor(content, cursor);
    content[..cursor]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

pub(crate) fn next_char_boundary(content: &str, cursor: usize) -> usize {
    let cursor = valid_cursor(content, cursor);
    content[cursor..]
        .chars()
        .next()
        .map(|character| cursor + character.len_utf8())
        .unwrap_or(content.len())
}

pub(crate) fn line_start(content: &str, cursor: usize) -> usize {
    let cursor = valid_cursor(content, cursor);
    content[..cursor]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0)
}

pub(crate) fn line_end(content: &str, cursor: usize) -> usize {
    let cursor = valid_cursor(content, cursor);
    content[cursor..]
        .find('\n')
        .map(|offset| cursor + offset)
        .unwrap_or(content.len())
}

pub(crate) fn move_cursor_vertically(content: &str, cursor: usize, direction: i8) -> usize {
    let cursor = valid_cursor(content, cursor);
    let current_start = line_start(content, cursor);
    let column = content[current_start..cursor].chars().count();
    let target_start = if direction < 0 {
        if current_start == 0 {
            return cursor;
        }
        line_start(content, current_start - 1)
    } else {
        let current_end = line_end(content, cursor);
        if current_end == content.len() {
            return cursor;
        }
        current_end + 1
    };
    let target_end = line_end(content, target_start);
    content[target_start..target_end]
        .char_indices()
        .map(|(offset, _)| target_start + offset)
        .nth(column)
        .unwrap_or(target_end)
}

#[cfg(test)]
pub(crate) fn insert(content: &mut String, cursor: &mut usize, text: &str) -> bool {
    *cursor = valid_cursor(content, *cursor);
    content.insert_str(*cursor, text);
    *cursor += text.len();
    !text.is_empty()
}

#[cfg(test)]
pub(crate) fn backspace(content: &mut String, cursor: &mut usize) -> bool {
    *cursor = valid_cursor(content, *cursor);
    let previous = previous_char_boundary(content, *cursor);
    if previous == *cursor {
        return false;
    }
    content.drain(previous..*cursor);
    *cursor = previous;
    true
}

#[cfg(test)]
pub(crate) fn delete(content: &mut String, cursor: &mut usize) -> bool {
    *cursor = valid_cursor(content, *cursor);
    let next = next_char_boundary(content, *cursor);
    if next == *cursor {
        return false;
    }
    content.drain(*cursor..next);
    true
}

pub(crate) fn with_visible_cursor(content: &str, cursor: usize) -> String {
    let mut visible = content.to_string();
    visible.insert(valid_cursor(&visible, cursor), '│');
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_respects_utf8_boundaries() {
        let mut content = "甲乙".to_string();
        let mut cursor = "甲".len();

        assert!(insert(&mut content, &mut cursor, "A"));
        assert_eq!(content, "甲A乙");
        assert_eq!(cursor, "甲A".len());

        assert!(backspace(&mut content, &mut cursor));
        assert_eq!(content, "甲乙");
        assert_eq!(cursor, "甲".len());

        assert!(delete(&mut content, &mut cursor));
        assert_eq!(content, "甲");
    }

    #[test]
    fn vertical_navigation_preserves_character_column() {
        let content = "ab甲\nx\n12345";
        let first_line_column_three = "ab甲".len();
        let second_line_end = move_cursor_vertically(content, first_line_column_three, 1);
        assert_eq!(second_line_end, "ab甲\nx".len());

        let third_line_column_one = move_cursor_vertically(content, second_line_end, 1);
        assert_eq!(third_line_column_one, "ab甲\nx\n1".len());
        assert_eq!(
            move_cursor_vertically(content, third_line_column_one, -1),
            second_line_end
        );
    }

    #[test]
    fn cursor_is_clamped_to_utf8_boundary() {
        assert_eq!(valid_cursor("甲", 2), 0);
        assert_eq!(with_visible_cursor("甲", 2), "│甲");
    }

    #[test]
    fn utf16_offsets_roundtrip_chinese_and_emoji() {
        let content = "中A😀文";
        for offset in [0, "中".len(), "中A".len(), "中A😀".len(), content.len()] {
            let utf16 = utf8_to_utf16_offset(content, offset);
            assert_eq!(utf16_to_utf8_offset(content, utf16), offset);
        }
    }

    #[test]
    fn composition_selection_is_relative_to_inserted_text() {
        let text = "拼音😀";
        let selected_utf16 = 1..4;
        let selected_utf8 = utf16_range_to_utf8(text, &selected_utf16);
        assert_eq!(&text[selected_utf8], "音😀");
    }
}
