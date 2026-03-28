//! Application orchestration.
//!
//! - `state.rs` — AppState, Mode, and pure data structs
//! - `actions.rs` — state mutations (testable without PTYs/async)
//! - `input.rs` — key/mouse → action translation

mod actions;
mod input;
pub mod state;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::Config;
use crate::events::AppEvent;
use crate::workspace::Workspace;

pub use state::{AppState, Mode, ToastKind, ViewState, CONTEXT_MENU_ITEMS};

/// Full application: AppState + runtime concerns (event channels, async I/O).
pub struct App {
    pub state: AppState,
    pub event_tx: mpsc::Sender<AppEvent>,
    event_rx: mpsc::Receiver<AppEvent>,
    no_session: bool,
    config_diagnostic_deadline: Option<Instant>,
    toast_deadline: Option<Instant>,
}

impl App {
    pub fn new(config: &Config, no_session: bool, config_diagnostic: Option<String>) -> Self {
        let (prefix_code, prefix_mods) = config.prefix_key();
        let (event_tx, event_rx) = mpsc::channel::<AppEvent>(64);

        // Try to restore previous session
        let (workspaces, active, selected) = if no_session {
            (Vec::new(), None, 0)
        } else if let Some(snap) = crate::persist::load() {
            let ws = crate::persist::restore(&snap, 24, 80, event_tx.clone());
            if ws.is_empty() {
                info!("session file found but no workspaces restored");
                (Vec::new(), None, 0)
            } else {
                info!(count = ws.len(), "session restored");
                let active = snap.active.filter(|&i| i < ws.len());
                let selected = snap.selected.min(ws.len().saturating_sub(1));
                (ws, active, selected)
            }
        } else {
            (Vec::new(), None, 0)
        };

        let mode = if config.should_show_onboarding() {
            state::Mode::Onboarding
        } else if active.is_some() {
            state::Mode::Terminal
        } else {
            state::Mode::Navigate
        };

        let state = AppState {
            workspaces,
            active,
            selected,
            mode,
            should_quit: false,
            request_new_workspace: false,
            request_complete_onboarding: false,
            name_input: String::new(),
            onboarding_step: 0,
            onboarding_selected: 1,
            view: state::ViewState {
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
            config_diagnostic,
            toast: None,
            prefix_code,
            prefix_mods,
            sidebar_width: config.ui.sidebar_width,
            sidebar_collapsed: false,
            confirm_close: config.ui.confirm_close,
            accent: crate::config::parse_color(&config.ui.accent),
            sound: config.ui.sound.clone(),
            toast_config: config.ui.toast.clone(),
            keybinds: config.keybinds(),
        };

        // Background auto-update (skipped in --no-session / test mode)
        if !no_session {
            let update_tx = event_tx.clone();
            std::thread::spawn(move || crate::update::auto_update(update_tx));
        }

        Self {
            config_diagnostic_deadline: state
                .config_diagnostic
                .as_ref()
                .map(|_| Instant::now() + Duration::from_secs(8)),
            toast_deadline: None,
            state,
            event_tx,
            event_rx,
            no_session,
        }
    }

    pub async fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.state.should_quit {
            if self
                .config_diagnostic_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.config_diagnostic_deadline = None;
                self.state.config_diagnostic = None;
            }

            if self
                .toast_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.toast_deadline = None;
                self.state.toast = None;
            }

            terminal.draw(|frame| {
                crate::ui::compute_view(&mut self.state, frame.area());
                crate::ui::render(&self.state, frame);
            })?;

            // Drain internal events
            while let Ok(ev) = self.event_rx.try_recv() {
                let previous_toast = self.state.toast.clone();
                self.state.handle_app_event(ev);
                if self.state.toast != previous_toast {
                    self.toast_deadline = self.state.toast.as_ref().map(|toast| {
                        let duration = match toast.kind {
                            ToastKind::NeedsAttention => Duration::from_secs(8),
                            ToastKind::Finished => Duration::from_secs(5),
                        };
                        Instant::now() + duration
                    });
                }
            }

            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.handle_key(key).await;
                    }
                    Event::Paste(text) => self.handle_paste(text).await,
                    Event::Mouse(mouse) => self.state.handle_mouse(mouse),
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }

            if self.state.request_complete_onboarding {
                self.state.request_complete_onboarding = false;
                self.complete_onboarding();
            }

            if self.state.request_new_workspace {
                self.state.request_new_workspace = false;
                self.create_workspace();
            }
        }

        // Save session on exit (skip in --no-session mode)
        if !self.no_session && !self.state.workspaces.is_empty() {
            let snap = crate::persist::capture(
                &self.state.workspaces,
                self.state.active,
                self.state.selected,
            );
            crate::persist::save(&snap);
        }

        Ok(())
    }

    pub(crate) fn complete_onboarding(&mut self) {
        let (sound_enabled, toast_enabled) = match self.state.onboarding_selected {
            0 => (false, false),
            1 => (false, true),
            2 => (true, false),
            _ => (true, true),
        };

        match crate::config::save_onboarding_choices(sound_enabled, toast_enabled) {
            Ok(()) => {
                self.state.sound.enabled = sound_enabled;
                self.state.toast_config.enabled = toast_enabled;
                self.state.mode = if self.state.active.is_some() {
                    Mode::Terminal
                } else {
                    Mode::Navigate
                };
            }
            Err(err) => {
                self.state.config_diagnostic =
                    Some(format!("failed to save onboarding config: {err}"));
                self.config_diagnostic_deadline = Some(Instant::now() + Duration::from_secs(8));
            }
        }
    }

    /// Create a workspace with a real PTY (needs event_tx).
    fn create_workspace(&mut self) {
        let (rows, cols) = self.state.estimate_pane_size();
        let initial_cwd = self
            .state
            .active
            .and_then(|i| self.state.workspaces.get(i))
            .and_then(|ws| ws.focused_runtime())
            .and_then(|rt| rt.cwd())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        match Workspace::new(initial_cwd, rows, cols, self.event_tx.clone()) {
            Ok(ws) => {
                self.state.workspaces.push(ws);
                let idx = self.state.workspaces.len() - 1;
                self.state.switch_workspace(idx);
                self.state.mode = Mode::Terminal;
            }
            Err(e) => {
                error!(err = %e, "failed to create workspace");
                self.state.mode = Mode::Navigate;
            }
        }
    }
}
