use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ratatui::layout::Direction;
use tokio::sync::mpsc;
use tracing::info;

use crate::detect::{self, Agent, AgentState};
use crate::events::AppEvent;
use crate::layout::{PaneId, TileLayout};
use crate::pane::{PaneRuntime, PaneState};

/// A named workspace containing tiled terminal panes.
pub struct Workspace {
    /// User-provided override. If set, auto-derived identity stops updating.
    pub custom_name: Option<String>,
    /// Identity source for this workspace.
    pub root_pane: PaneId,
    pub layout: TileLayout,
    /// Cached ahead/behind counts for the root repo's current branch upstream.
    pub(crate) cached_git_ahead_behind: Option<(usize, usize)>,
    /// Stable-ish public pane numbers within this workspace.
    /// New panes append at the end; closing a pane compacts higher numbers down.
    pub public_pane_numbers: HashMap<PaneId, usize>,
    pub(crate) next_public_pane_number: usize,
    /// Pane state — always present, testable without PTYs.
    pub panes: HashMap<PaneId, PaneState>,
    /// Pane runtimes — only present in production (empty in tests).
    pub runtimes: HashMap<PaneId, PaneRuntime>,
    pub zoomed: bool,
    pub events: mpsc::Sender<AppEvent>,
}

impl Workspace {
    pub fn new(
        initial_cwd: PathBuf,
        rows: u16,
        cols: u16,
        events: mpsc::Sender<AppEvent>,
    ) -> std::io::Result<Self> {
        let (layout, root_id) = TileLayout::new();
        let runtime = PaneRuntime::spawn(root_id, rows, cols, initial_cwd, events.clone())?;

        let mut panes = HashMap::new();
        panes.insert(root_id, PaneState::new());
        let mut public_pane_numbers = HashMap::new();
        public_pane_numbers.insert(root_id, 1);
        let mut runtimes = HashMap::new();
        runtimes.insert(root_id, runtime);

        info!(root_pane = root_id.raw(), "workspace created");
        Ok(Self {
            custom_name: None,
            root_pane: root_id,
            layout,
            cached_git_ahead_behind: None,
            public_pane_numbers,
            next_public_pane_number: 2,
            panes,
            runtimes,
            zoomed: false,
            events,
        })
    }

    /// Split the focused pane. Returns the new pane id.
    pub fn split_focused(
        &mut self,
        direction: Direction,
        rows: u16,
        cols: u16,
        cwd: Option<PathBuf>,
    ) -> std::io::Result<PaneId> {
        let new_id = self.layout.split_focused(direction);
        let actual_cwd =
            cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let runtime = PaneRuntime::spawn(new_id, rows, cols, actual_cwd, self.events.clone())?;
        self.panes.insert(new_id, PaneState::new());
        self.public_pane_numbers
            .insert(new_id, self.next_public_pane_number);
        self.next_public_pane_number += 1;
        self.runtimes.insert(new_id, runtime);
        self.zoomed = false;
        Ok(new_id)
    }

    /// Close the focused pane. Returns the removed pane id, or None if last pane.
    pub fn close_focused(&mut self) -> Option<PaneId> {
        let pane_id = self.layout.focused();
        self.remove_pane(pane_id)
    }

    /// Remove a specific pane from this workspace.
    /// Returns None if it's the last pane and the whole workspace should close.
    pub fn remove_pane(&mut self, pane_id: PaneId) -> Option<PaneId> {
        if self.layout.pane_count() <= 1 {
            return None;
        }

        let next_root = self.promoted_root_if_needed(pane_id);

        if self.layout.focused() == pane_id {
            self.layout.close_focused();
        } else {
            let prev_focus = self.layout.focused();
            self.layout.focus_pane(pane_id);
            self.layout.close_focused();
            self.layout.focus_pane(prev_focus);
        }

        if let Some(removed_number) = self.public_pane_numbers.remove(&pane_id) {
            for number in self.public_pane_numbers.values_mut() {
                if *number > removed_number {
                    *number -= 1;
                }
            }
            self.next_public_pane_number = self.public_pane_numbers.len() + 1;
        }
        self.panes.remove(&pane_id);
        self.runtimes.remove(&pane_id);
        self.zoomed = false;
        if let Some(next_root) = next_root {
            self.root_pane = next_root;
        }
        Some(pane_id)
    }

    fn promoted_root_if_needed(&self, closing: PaneId) -> Option<PaneId> {
        if self.root_pane != closing {
            return None;
        }
        self.layout.pane_ids().into_iter().find(|id| *id != closing)
    }

    pub fn public_pane_number(&self, pane_id: PaneId) -> Option<usize> {
        self.public_pane_numbers.get(&pane_id).copied()
    }

    /// Get the runtime for the focused pane.
    pub fn focused_runtime(&self) -> Option<&PaneRuntime> {
        self.runtimes.get(&self.layout.focused())
    }

    pub fn set_custom_name(&mut self, name: String) {
        self.custom_name = Some(name);
    }

    pub fn display_name(&self) -> String {
        if let Some(name) = &self.custom_name {
            return name.clone();
        }

        self.root_cwd()
            .as_deref()
            .map(derive_label_from_cwd)
            .unwrap_or_else(|| "shell".to_string())
    }

    pub fn root_cwd(&self) -> Option<PathBuf> {
        self.runtimes.get(&self.root_pane).and_then(|rt| rt.cwd())
    }

    /// Agent name summary (e.g. "claude, amp"). Used by collapsed sidebar.
    pub fn agent_summary(&self) -> Option<String> {
        let mut names = Vec::new();

        for id in self.layout.pane_ids() {
            if let Some(agent) = self.panes.get(&id).and_then(|p| p.detected_agent) {
                let name = agent_name(agent);
                if !names.iter().any(|n| *n == name) {
                    names.push(name);
                }
            }
        }

        if names.is_empty() {
            if self.root_cwd().is_some() {
                Some("shell".to_string())
            } else {
                None
            }
        } else if names.len() == 1 {
            Some(names[0].to_string())
        } else if names.len() == 2 {
            Some(format!("{}, {}", names[0], names[1]))
        } else {
            Some(format!("{}, {} +{}", names[0], names[1], names.len() - 2))
        }
    }

    /// Aggregate state + seen across all panes.
    /// Returns the highest-priority state and the worst-case seen flag.
    pub fn aggregate_state(&self) -> (AgentState, bool) {
        let states: Vec<AgentState> = self.panes.values().map(|p| p.state).collect();
        let state = detect::workspace_state(&states);
        let seen = self.panes.values().all(|p| p.seen);
        (state, seen)
    }

    /// Per-pane (state, seen) in BSP tree order (left-to-right, top-to-bottom).
    pub fn pane_states(&self) -> Vec<(AgentState, bool)> {
        self.layout
            .pane_ids()
            .iter()
            .map(|id| {
                self.panes
                    .get(id)
                    .map(|p| (p.state, p.seen))
                    .unwrap_or((AgentState::Unknown, true))
            })
            .collect()
    }

    /// Per-pane detail for the agent detail panel, ordered by urgency.
    pub fn pane_details(&self) -> Vec<PaneDetail> {
        let mut details: Vec<PaneDetail> = self
            .layout
            .pane_ids()
            .iter()
            .map(|id| {
                let pane = self.panes.get(id);
                let agent = pane.and_then(|p| p.detected_agent);
                let state = pane.map(|p| p.state).unwrap_or(AgentState::Unknown);
                let seen = pane.map(|p| p.seen).unwrap_or(true);
                let label = agent
                    .map(|a| agent_name(a).to_string())
                    .unwrap_or_else(|| "shell".to_string());
                PaneDetail {
                    label,
                    agent,
                    state,
                    seen,
                }
            })
            .collect();
        // Sort by urgency: blocked > working > idle(unseen) > idle > unknown
        details.sort_by(|a, b| b.urgency().cmp(&a.urgency()));
        details
    }

    /// Get the git branch for this workspace's root cwd.
    pub fn branch(&self) -> Option<String> {
        self.root_cwd().and_then(|cwd| git_branch(&cwd))
    }

    /// Cached ahead/behind counts for this workspace's current branch upstream.
    pub fn git_ahead_behind(&self) -> Option<(usize, usize)> {
        self.cached_git_ahead_behind
    }

    /// Refresh cached ahead/behind counts from the workspace's current cwd.
    pub fn refresh_git_ahead_behind(&mut self) {
        self.cached_git_ahead_behind = self.root_cwd().and_then(|cwd| git_ahead_behind(&cwd));
    }
}

/// Detail info for a single pane, used by the agent detail panel.
pub struct PaneDetail {
    pub label: String,
    /// The detected agent, if any. Will be used for context extraction.
    #[allow(dead_code)] // used later for triage line extraction
    pub agent: Option<Agent>,
    pub state: AgentState,
    pub seen: bool,
}

impl PaneDetail {
    /// Urgency score for sort ordering (higher = more urgent, shown first).
    fn urgency(&self) -> u8 {
        match (self.state, self.seen) {
            (AgentState::Blocked, _) => 4,
            (AgentState::Working, _) => 3,
            (AgentState::Idle, false) => 2, // unseen/done
            (AgentState::Idle, true) => 1,
            (AgentState::Unknown, _) => 0,
        }
    }
}

fn agent_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Pi => "pi",
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Gemini => "gemini",
        Agent::Cursor => "cursor",
        Agent::Cline => "cline",
        Agent::OpenCode => "opencode",
        Agent::GithubCopilot => "copilot",
        Agent::Kimi => "kimi",
        Agent::Droid => "droid",
        Agent::Amp => "amp",
    }
}

fn derive_label_from_cwd(cwd: &Path) -> String {
    if let Some(repo_root) = git_repo_root(cwd) {
        if let Some(name) = repo_root.file_name().and_then(|n| n.to_str()) {
            return name.to_string();
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let home = Path::new(&home);
        if cwd == home {
            return "~".to_string();
        }
    }

    cwd.file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| cwd.display().to_string())
}

/// Read the current git branch name from .git/HEAD.
/// Returns None if not in a git repo or HEAD is detached.
pub fn git_branch(cwd: &Path) -> Option<String> {
    let repo_root = git_repo_root(cwd)?;
    let head_path = repo_root.join(".git").join("HEAD");
    let content = std::fs::read_to_string(head_path).ok()?;
    let trimmed = content.trim();
    trimmed
        .strip_prefix("ref: refs/heads/")
        .map(|s| s.to_string())
}

fn git_repo_root(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };

    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Read ahead/behind counts relative to the current branch upstream.
fn git_ahead_behind(cwd: &Path) -> Option<(usize, usize)> {
    git_repo_root(cwd)?;

    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_git_ahead_behind_output(&stdout)
}

fn parse_git_ahead_behind_output(stdout: &str) -> Option<(usize, usize)> {
    let mut parts = stdout.split_whitespace();
    let ahead = parts.next()?.parse().ok()?;
    let behind = parts.next()?.parse().ok()?;
    Some((ahead, behind))
}

// ---------------------------------------------------------------------------
// Test helpers — construct workspaces without PTYs
// ---------------------------------------------------------------------------

#[cfg(test)]
impl Workspace {
    /// Create a test workspace with one pane, no PTY runtime.
    pub fn test_new(name: &str) -> Self {
        let (events, _) = mpsc::channel(64);
        let (layout, root_id) = TileLayout::new();
        let mut panes = HashMap::new();
        panes.insert(root_id, PaneState::new());
        let mut public_pane_numbers = HashMap::new();
        public_pane_numbers.insert(root_id, 1);
        Self {
            custom_name: Some(name.to_string()),
            root_pane: root_id,
            layout,
            cached_git_ahead_behind: None,
            public_pane_numbers,
            next_public_pane_number: 2,
            panes,
            runtimes: HashMap::new(),
            zoomed: false,
            events,
        }
    }

    /// Add a test pane (splits focused, no PTY runtime).
    pub fn test_split(&mut self, direction: Direction) -> PaneId {
        let new_id = self.layout.split_focused(direction);
        self.panes.insert(new_id, PaneState::new());
        self.public_pane_numbers
            .insert(new_id, self.next_public_pane_number);
        self.next_public_pane_number += 1;
        new_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::AgentState;

    #[test]
    fn aggregate_state_all_unknown() {
        let ws = Workspace::test_new("test");
        let (state, seen) = ws.aggregate_state();
        assert_eq!(state, AgentState::Unknown);
        assert!(seen);
    }

    #[test]
    fn aggregate_state_priority() {
        let mut ws = Workspace::test_new("test");
        let id2 = ws.test_split(Direction::Horizontal);

        let root_id = *ws.panes.keys().find(|id| **id != id2).unwrap();
        ws.panes.get_mut(&root_id).unwrap().state = AgentState::Idle;
        ws.panes.get_mut(&id2).unwrap().state = AgentState::Working;

        let (state, _) = ws.aggregate_state();
        assert_eq!(state, AgentState::Working);
    }

    #[test]
    fn aggregate_seen_any_unseen_means_unseen() {
        let mut ws = Workspace::test_new("test");
        let id2 = ws.test_split(Direction::Horizontal);

        ws.panes.get_mut(&id2).unwrap().seen = false;

        let (_, seen) = ws.aggregate_state();
        assert!(!seen);
    }

    #[test]
    fn close_focused_removes_pane() {
        let mut ws = Workspace::test_new("test");
        let _id2 = ws.test_split(Direction::Horizontal);
        assert_eq!(ws.panes.len(), 2);

        let closed = ws.close_focused();
        assert!(closed.is_some());
        assert_eq!(ws.panes.len(), 1);
    }

    #[test]
    fn close_focused_last_pane_returns_none() {
        let mut ws = Workspace::test_new("test");
        assert_eq!(ws.panes.len(), 1);

        let closed = ws.close_focused();
        assert!(closed.is_none());
        assert_eq!(ws.panes.len(), 1);
    }

    #[test]
    fn pane_states_matches_layout_order() {
        let mut ws = Workspace::test_new("test");
        let id2 = ws.test_split(Direction::Horizontal);

        ws.panes.get_mut(&id2).unwrap().state = AgentState::Blocked;

        let states = ws.pane_states();
        assert_eq!(states.len(), 2);
        assert!(states.iter().any(|(s, _)| *s == AgentState::Blocked));
        assert!(states.iter().any(|(s, _)| *s == AgentState::Unknown));
    }

    #[test]
    fn closing_root_promotes_another_pane() {
        let mut ws = Workspace::test_new("test");
        let root = ws.root_pane;
        let other = ws.test_split(Direction::Horizontal);
        ws.layout.focus_pane(root);
        ws.remove_pane(root);
        assert_eq!(ws.root_pane, other);
    }

    #[test]
    fn parse_git_ahead_behind_output_maps_first_field_to_ahead() {
        assert_eq!(parse_git_ahead_behind_output("7\t0\n"), Some((7, 0)));
    }

    #[test]
    fn parse_git_ahead_behind_output_maps_second_field_to_behind() {
        assert_eq!(parse_git_ahead_behind_output("0 3\n"), Some((0, 3)));
    }
}
