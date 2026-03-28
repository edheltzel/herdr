# Changelog

## Unreleased

### Added
- Added first-run onboarding flow that lets you choose notification preferences (sound and toast) on startup.
- Added optional visual toast notifications in the top-right corner for background workspace events (completion and attention-needed alerts).
- Added configurable keybindings for all navigate mode actions: new workspace, rename workspace, close workspace, resize mode, and toggle sidebar. See `CONFIGURATION.md` for the full key reference.
- Added configuration validation with startup diagnostics. Invalid key combinations or duplicate bindings now fall back to safe defaults with a visible warning.

### Changed
- **Breaking:** Default prefix key changed from `ctrl+s` to `ctrl+b` to avoid common terminal flow control conflicts.
- Workspaces now derive their identity from the repository or folder of their root pane, updating automatically as you navigate. Custom names act as overrides rather than static labels.
- Sidebar now shows workspace numbers again in expanded view.
- Refined sidebar presentation with consistent marker/name/state ordering and comma-separated agent summaries.
- Keybinding parser now accepts special keys (`enter`, `esc`, `tab`, `backspace`, `space`) and function keys (`f1`–`f12`).

### Documentation
- Split configuration reference into a dedicated `CONFIGURATION.md` file with full keybinding documentation and config diagnostics explanation.

## [0.1.1] - 2026-03-28

### Added
- Added optional sound notifications for agent state changes, including a completion chime when background work finishes and an alert when an agent needs input.
- Added per-agent sound overrides under `[ui.sound.agents]`, so you can mute or enable notifications by agent instead of using one global setting. Droid notifications are muted by default.

### Changed
- Request alerts now play even when the agent is in the active workspace, while completion sounds remain limited to background workspaces.

### Fixed
- Improved foreground job detection on Linux and macOS so herdr can recognize agents that run through wrapper processes or generic runtimes, including cases like Codex running under `node`.
- Made Claude Code state detection more stable by handling more spinner variants and smoothing short busy/idle flicker during screen updates.

## [0.1.0] - 2026-03-27

### Added
- Initial release.
