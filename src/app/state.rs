use crate::config::{Keybinds, SoundConfig, ToastConfig};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Direction, Rect};
use ratatui::style::Color;

use crate::layout::{PaneInfo, SplitBorder};
use crate::selection::Selection;
use crate::workspace::Workspace;

/// Computed view geometry — derived from AppState + terminal size.
/// Updated before each render, consumed by render and mouse handling.
pub struct ViewState {
    pub sidebar_rect: Rect,
    pub terminal_area: Rect,
    pub pane_infos: Vec<PaneInfo>,
    pub split_borders: Vec<SplitBorder>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Onboarding,
    Navigate,
    Terminal,
    RenameSession,
    Resize,
    ConfirmClose,
    ContextMenu,
}

/// Active mouse drag on a split border.
pub(crate) struct DragState {
    pub path: Vec<bool>,
    pub direction: Direction,
    pub area: Rect,
}

/// Right-click context menu state.
pub struct ContextMenuState {
    pub ws_idx: usize,
    pub x: u16,
    pub y: u16,
    pub selected: usize,
}

pub const CONTEXT_MENU_ITEMS: &[&str] = &["Rename", "Close"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    NeedsAttention,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastNotification {
    pub kind: ToastKind,
    pub title: String,
    pub context: String,
}

/// All application state — pure data, no channels or async runtime.
/// Testable without PTYs or a tokio runtime.
pub struct AppState {
    pub workspaces: Vec<Workspace>,
    pub active: Option<usize>,
    pub selected: usize,
    pub mode: Mode,
    pub should_quit: bool,
    pub request_new_workspace: bool,
    pub request_complete_onboarding: bool,
    pub name_input: String,
    pub onboarding_step: usize,
    pub onboarding_selected: usize,
    // View geometry (computed before render, consumed by render + mouse)
    pub view: ViewState,
    pub(crate) drag: Option<DragState>,
    pub selection: Option<Selection>,
    pub context_menu: Option<ContextMenuState>,
    // Notifications
    pub update_available: Option<String>,
    pub update_dismissed: bool,
    pub config_diagnostic: Option<String>,
    pub toast: Option<ToastNotification>,
    // Config
    pub prefix_code: KeyCode,
    pub prefix_mods: KeyModifiers,
    pub sidebar_width: u16,
    pub sidebar_collapsed: bool,
    pub confirm_close: bool,
    pub accent: Color,
    pub sound: SoundConfig,
    pub toast_config: ToastConfig,
    pub keybinds: Keybinds,
}

impl AppState {
    pub fn is_prefix(&self, key: &crossterm::event::KeyEvent) -> bool {
        key_matches(key, self.prefix_code, self.prefix_mods)
    }

    pub fn estimate_pane_size(&self) -> (u16, u16) {
        if let Some(info) = self.view.pane_infos.first() {
            (info.rect.height, info.rect.width)
        } else {
            (24, 80)
        }
    }
}

pub fn key_matches(
    key: &crossterm::event::KeyEvent,
    expected_code: KeyCode,
    expected_mods: KeyModifiers,
) -> bool {
    if key.modifiers != expected_mods {
        return false;
    }

    match (key.code, expected_code) {
        (KeyCode::Char(actual), KeyCode::Char(expected))
            if actual.is_ascii_alphabetic() && expected.is_ascii_alphabetic() =>
        {
            actual.eq_ignore_ascii_case(&expected)
        }
        (actual, expected) => actual == expected,
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
impl AppState {
    /// Create an AppState for testing — no channels, no PTYs.
    pub fn test_new() -> Self {
        Self {
            workspaces: Vec::new(),
            active: None,
            selected: 0,
            mode: Mode::Navigate,
            should_quit: false,
            request_new_workspace: false,
            request_complete_onboarding: false,
            name_input: String::new(),
            onboarding_step: 0,
            onboarding_selected: 1,
            view: ViewState {
                sidebar_rect: Rect::default(),
                terminal_area: Rect::default(),
                pane_infos: Vec::new(),
                split_borders: Vec::new(),
            },
            drag: None,
            selection: None,
            context_menu: None,
            update_available: None,
            update_dismissed: false,
            config_diagnostic: None,
            toast: None,
            prefix_code: KeyCode::Char('b'),
            prefix_mods: KeyModifiers::CONTROL,
            sidebar_width: 26,
            sidebar_collapsed: false,
            confirm_close: true,
            accent: Color::Cyan,
            sound: SoundConfig {
                enabled: false,
                ..SoundConfig::default()
            },
            toast_config: ToastConfig::default(),
            keybinds: Keybinds {
                new_workspace: (KeyCode::Char('n'), KeyModifiers::empty()),
                new_workspace_label: "n".into(),
                rename_workspace: (KeyCode::Char('n'), KeyModifiers::SHIFT),
                rename_workspace_label: "shift+n".into(),
                close_workspace: (KeyCode::Char('d'), KeyModifiers::empty()),
                close_workspace_label: "d".into(),
                split_vertical: (KeyCode::Char('v'), KeyModifiers::empty()),
                split_vertical_label: "v".into(),
                split_horizontal: (KeyCode::Char('-'), KeyModifiers::empty()),
                split_horizontal_label: "-".into(),
                close_pane: (KeyCode::Char('x'), KeyModifiers::empty()),
                close_pane_label: "x".into(),
                fullscreen: (KeyCode::Char('f'), KeyModifiers::empty()),
                fullscreen_label: "f".into(),
                resize_mode: (KeyCode::Char('r'), KeyModifiers::empty()),
                resize_mode_label: "r".into(),
                toggle_sidebar: (KeyCode::Char('b'), KeyModifiers::empty()),
                toggle_sidebar_label: "b".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn key_matches_requires_exact_modifiers() {
        assert!(key_matches(
            &KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ));

        assert!(!key_matches(
            &KeyEvent::new(
                KeyCode::Char('b'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ));
    }

    #[test]
    fn key_matches_letters_case_insensitively() {
        assert!(key_matches(
            &KeyEvent::new(KeyCode::Char('B'), KeyModifiers::SHIFT),
            KeyCode::Char('b'),
            KeyModifiers::SHIFT,
        ));
    }
}
