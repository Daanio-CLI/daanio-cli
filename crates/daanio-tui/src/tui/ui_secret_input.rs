//! Credential-safe composer projections.

use super::TuiState;
use std::borrow::Cow;

pub(super) fn visible_input(app: &dyn TuiState) -> (Cow<'_, str>, usize) {
    if app.input_is_secret() {
        let cursor = crate::tui::core::byte_offset_to_char_index(app.input(), app.cursor_pos());
        (Cow::Owned("*".repeat(app.input().chars().count())), cursor)
    } else {
        (Cow::Borrowed(app.input()), app.cursor_pos())
    }
}

pub(super) fn debug_preview(app: &dyn TuiState) -> String {
    if app.input_is_secret() {
        "<secret input hidden>".to_string()
    } else {
        app.input().chars().take(100).collect()
    }
}
