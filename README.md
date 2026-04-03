<p align="center">
  <img src="assets/logo.png" alt="herdr" width="120" />
</p>

<h1 align="center">herdr</h1>

<p align="center">herd your agents.</p>

<p align="center">
  <a href="https://herdr.dev">herdr.dev</a> · <a href="#install">install</a> · <a href="#usage">usage</a> · <a href="./INTEGRATIONS.md">integrations</a> · <a href="./CONFIGURATION.md">configuration</a> · <a href="./SKILL.md">agent skill</a> · <a href="./SOCKET_API.md">socket api</a>
</p>

---

herdr is a terminal-native agent multiplexer for coding agents.

it runs inside your existing terminal: ghostty, alacritty, kitty, wezterm, even inside tmux. a single rust binary that gives you workspaces, tiled panes, automatic agent detection, and notification alerts without asking you to leave the terminal for a separate gui window, electron wrapper, or web dashboard.

it is also becoming a shared control surface. you manage agents in herdr, and increasingly those agents can interact with herdr too through its local socket api, cli commands, and the example agent skill in this repo.

https://github.com/user-attachments/assets/ea508dcd-67ea-4cc7-9e4d-eb252f37c196

## what herdr is

most tools in this space try to replace your environment.

herdr takes the opposite approach. it lives where cli agents already live, keeps mouse and keyboard both first-class, and adds the missing layer: awareness and coordination for running multiple agents in parallel.

that means two things:

- **for you:** one terminal-native place to supervise multiple agents, jump between contexts, and notice when something needs attention.
- **for your agents:** an automation surface they can increasingly use themselves to create panes, spawn other agents, run helpers, read output, and wait on state.

## workspace model

herdr is opinionated about workspaces.

it does **not** start by asking you to create an empty named project or pick a folder in a setup flow. you create a workspace, and it opens immediately as a new terminal context.

from there, the workspace identity comes from its **root pane**:

- the first pane in a workspace is the root pane
- it starts as the top-left anchor and owns the workspace identity
- the workspace label defaults to the root pane's git repo name
- if there is no git repo, it falls back to the root pane's current folder name
- manual rename is supported, but it is an override, not the primary model

this keeps workspaces lightweight. they are project contexts first, labels second.

## awareness and notifications

the sidebar is split into two layers:

- **top:** workspaces, each with one aggregate state dot
- **bottom:** detected agents inside the selected or active workspace

herdr automatically detects running agents by looking at the foreground process and reading terminal output. the top section compresses each workspace into one prioritized signal so you can scan the whole workspace list quickly; the bottom section shows which specific agent is causing it.

workspace and agent states map to:

- 🔴 **blocked** — agent needs input or approval
- 🔵 **done** — work finished and you have not looked at it yet
- 🟡 **working** — agent is actively running
- 🟢 **idle** — done, seen, and calm
- ⚪ **unknown** — plain shell or undetected

workspace rollups prefer the most urgent thing happening in that workspace: blocked first, then unseen finished work, then working, then idle.

plain shells still contribute to workspace rollups, but the sidebar's agent section intentionally hides non-agent panes so the detail list stays focused on actual agents.

if you want more interruption than ambient sidebar awareness, herdr can also play sounds or show top-right toast notifications for background events. notification suppression is tab-aware: the active tab stays quiet, but background tabs in the same workspace can still alert.

## agents can use herdr too

herdr is not just a passive manager for humans watching agents. it gives agents two clean integration paths:

- [`SKILL.md`](./SKILL.md) — the reusable agent skill. use this if you want an agent inside herdr to learn the workflow quickly through the existing cli surface.
- [`SOCKET_API.md`](./SOCKET_API.md) — the direct integration doc. use this if you want the low-level socket protocol, event subscriptions, or the cli wrapper reference that sits on top of it.

those two paths meet at the same control surface. the built-in `herdr workspace ...`, `herdr tab ...`, `herdr pane ...`, and `herdr wait ...` commands all talk to the same local socket api.

that means agents running inside herdr can do useful orchestration work themselves:

- create new workspaces for parallel tasks
- create and focus tabs inside a workspace
- split panes for servers, logs, tests, or scratch work
- spawn other agents in sibling panes
- read pane output or wait for output matches
- send text and keys into other panes
- wait for another agent to finish before continuing

in that sense, herdr is for you **and** for your agents.

## supported agents

herdr detects agent state by identifying the foreground process and reading terminal output patterns. the following agents have been tested:

| agent | idle / done | working | blocked |
|-------|-------------|---------|---------|
| [pi](https://pi.dev) | ✓ | ✓ | partial |
| [claude code](https://docs.anthropic.com/en/docs/claude-code) | ✓ | ✓ | ✓ |
| [codex](https://github.com/openai/codex) | ✓ | ✓ | ✓ |
| [droid](https://factory.ai) | ✓ | ✓ | ✓ |
| [amp](https://ampcode.com) | ✓ | ✓ | partial |
| [opencode](https://github.com/anomalyco/opencode) | ✓ | ✓ | ✓ |

detection heuristics also exist for these agents but have not been fully tested yet. if you use them and run into issues, please [open an issue](https://github.com/ogulcancelik/herdr/issues):

- [gemini cli](https://github.com/google-gemini/gemini-cli)
- [cursor agent](https://cursor.com/cli)
- [cline](https://github.com/cline/cline)
- [kimi](https://kimi.ai)
- [github copilot cli](https://cli.github.com)

for any other cli agent, herdr still works as a terminal-native multiplexer. you still get workspaces, panes, tiling, and notifications. richer direct agent-side reporting on top of the socket/event layer is still evolving.

## install

```bash
curl -fsSL https://herdr.dev/install.sh | sh
```

or download the binary directly from [releases](https://github.com/ogulcancelik/herdr/releases).

requirements: linux or macos.

### update

herdr checks for updates automatically in the background. when a new version is ready, you'll see a notification in the ui. just restart to apply. you can also update manually:

```bash
herdr update
```

## usage

launch herdr:

```bash
herdr
```

on first run, herdr opens a short onboarding flow so you can choose your notification style. after that, if a session is restored you'll land in terminal mode; otherwise you'll start in **navigate mode**.

press `n` to create your first workspace. it opens immediately as a new terminal context in your current project context, and herdr labels it automatically from the root pane's repo or folder.

press `ctrl+b` (the prefix key) to switch back to navigate mode. from there you can manage workspaces, tabs, and panes.

### navigate mode (prefix: ctrl+b)

navigate mode is the workspace control layer. a workspace can contain multiple tabs, and each tab can contain multiple panes. movement actions stay in navigate mode; mutating actions like split, close, new workspace, new tab, and sidebar toggle return you to terminal mode.

common defaults:
- `n` new workspace
- `shift+n` rename workspace
- `d` close workspace
- `c` new tab
- `v` / `-` split pane
- `x` close pane
- `f` fullscreen
- `r` resize mode
- `b` toggle sidebar

optional direct bindings are available but **unset by default**. you can bind workspace, tab, and pane switching directly in terminal mode without going through the prefix first.

example:

```toml
[keys]
previous_workspace = "ctrl+alt+["
next_workspace = "ctrl+alt+]"
previous_tab = "alt+["
next_tab = "alt+]"
focus_pane_left = "alt+h"
focus_pane_down = "alt+j"
focus_pane_up = "alt+k"
focus_pane_right = "alt+l"
```

full keybinding and config reference: [`CONFIGURATION.md`](./CONFIGURATION.md)

### sidebar

the sidebar is your triage surface.

- the top section tells you which workspace needs attention
- the bottom section tells you which agent inside that workspace is causing it

this is the core loop of herdr: scan the workspace list, drop into the right context, then act.

### resize mode

| key | action |
|-----|--------|
| `h` `l` | resize width |
| `j` `k` | resize height |
| `esc` | exit resize mode |

### mouse

mouse support is built in. herdr is not keyboard-only.

- click a workspace in the sidebar to switch
- click tabs to switch within the active workspace
- click a pane to focus it
- drag split borders to resize
- drag in a pane to select text; release to copy it to your system clipboard
- right-click a workspace for context menu
- scroll in sidebar to navigate workspaces
- click `«` / `»` at the sidebar bottom to collapse/expand

text copy uses OSC 52, so it depends on your terminal's clipboard support.

### terminal mode

you're in a real terminal. everything works: your shell, vim, htop, ssh, anything. press the prefix key (`ctrl+b`) to go back to navigate mode.

## configuration

config file: `~/.config/herdr/config.toml`

print the full default config with:

```bash
herdr --default-config
```

### themes

herdr ships with 9 built-in themes: catppuccin (default), tokyo night, dracula, nord, gruvbox, one dark, solarized, kanagawa, and rosé pine.

```toml
[theme]
name = "tokyo-night"
```

you can also override individual color tokens on top of any base theme. see [`CONFIGURATION.md`](./CONFIGURATION.md) for the full token reference.

for all keybindings, onboarding, notification, sound, ui options, and environment variables, see [`CONFIGURATION.md`](./CONFIGURATION.md).

## session persistence

herdr saves your workspaces, tabs, pane layouts, pane working directories, and focused tab/pane on exit. when you restart, everything is restored. sessions are stored at `~/.config/herdr/session.json`.

through the socket api, tabs are first-class too now. raw integrations and cli wrappers can list/create/focus/rename/close tabs directly while pane ids stay workspace-scoped.

use `--no-session` to start fresh.

## how agent detection works

herdr does not require hooks or agent-side configuration for its built-in detection. it works by:

1. identifying the foreground process of each pane's pty (via `/proc` on linux, `proc_pidinfo` on macos)
2. matching the process name against known agents
3. reading terminal screen content and applying per-agent heuristics to determine state

this means detection works with any supported agent, installed any way, with zero setup. if it runs in a terminal, herdr can see it.

the heuristics are pattern-matched against each agent's actual terminal output: prompt boxes, spinners, waiting-for-input messages, tool execution indicators. detection runs on a separate async task per pane, polled every 300-500ms, decoupled from terminal rendering.

## optional direct integrations

herdr also supports optional direct integrations for tools that expose hooks or plugins:

- [pi](./INTEGRATIONS.md#pi)
- [claude code](./INTEGRATIONS.md#claude-code)
- [codex](./INTEGRATIONS.md#codex)
- [opencode](./INTEGRATIONS.md#opencode)

install them with:

```bash
herdr integration install pi
herdr integration install claude
herdr integration install codex
herdr integration install opencode
```

remove them with:

```bash
herdr integration uninstall pi
herdr integration uninstall claude
herdr integration uninstall codex
herdr integration uninstall opencode
```

these integrations improve semantic state reporting, but they do not replace herdr's core process detection model. for setup details, file locations, caveats, and uninstall behavior, see [`INTEGRATIONS.md`](./INTEGRATIONS.md).

known codex caveat: codex currently renders hook lifecycle lines in its own tui when hooks are enabled. that noise is upstream codex behavior, not herdr-specific.

## api and automation

for direct integration details, use the docs instead of reverse-engineering the README:

- [`INTEGRATIONS.md`](./INTEGRATIONS.md) — install and behavior notes for pi, claude code, codex, and opencode
- [`SKILL.md`](./SKILL.md) — reusable agent skill for agents already running inside herdr
- [`SOCKET_API.md`](./SOCKET_API.md) — canonical socket protocol + cli wrapper reference

`SOCKET_API.md` now covers transport, request/response envelopes, workspace, tab, and pane methods, subscription behavior, and the `herdr workspace`, `herdr tab`, `herdr pane`, and `herdr wait` commands that wrap the same socket surface.

## what's coming

- **richer agent-side hooks**: better ways for unsupported tools and custom workflows to report state directly into herdr.
- **deeper orchestration helpers**: more event-driven and shell-friendly wrappers on top of the socket foundation.
- **in-app preferences**: rerun onboarding and adjust things like sound and toast notifications without editing config by hand.
- **native notifications**: os-level notifications when an agent needs attention and herdr is not in focus.

## built with agents

i had never written rust before starting this project. herdr was built almost entirely through ai coding agents, the same ones it is designed to multiplex. i supervised the architecture and specs; agents wrote the code.

this is a proof of concept in more ways than one. it is a functional tool, but it is also a statement about what is possible right now. if you can build a terminal-native agent multiplexer in a language you do not know, by directing the same agents the tool is built for, that says something about where we are.

there will be rough edges. if you hit one, [open an issue](https://github.com/ogulcancelik/herdr/issues). that's why it's open source.

## cli wrappers

herdr's workspace, tab, pane, and wait commands are documented in [`SOCKET_API.md`](./SOCKET_API.md) together with the socket methods they wrap.

workspace ids are compact public ids like `1`, `2`, `3`.
tab ids are compact public ids like `1:1`, `1:2`, `2:1`.
pane ids are compact public ids like `1-1`, `1-2`, `2-1`.

even with tabs enabled, pane ids remain workspace-scoped public ids rather than `workspace-tab-pane` triples. both tab ids and pane ids are positional within the current live session, so numbering compacts when tabs, workspaces, or panes are closed.

## building from source

```bash
git clone https://github.com/ogulcancelik/herdr
cd herdr
cargo build --release
./target/release/herdr
```

## testing

```bash
just test               # unit tests
just test-all           # full local test suite
```

## license

AGPL-3.0: free to use, modify, and distribute. if you distribute a modified version, you must open-source your changes under the same license.
