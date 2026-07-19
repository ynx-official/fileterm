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

pub(crate) fn insert(content: &mut String, cursor: &mut usize, text: &str) -> bool {
    *cursor = valid_cursor(content, *cursor);
    content.insert_str(*cursor, text);
    *cursor += text.len();
    !text.is_empty()
}

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
}
