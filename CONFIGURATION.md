# configuration

herdr reads config from:

```text
~/.config/herdr/config.toml
```

print the full default config with:

```bash
herdr --default-config
```

if a config value is invalid, or two navigate actions use the same keybinding, herdr falls back to a safe default and shows a startup warning in the UI.

## onboarding

```toml
onboarding = true
```

| option | default | description |
|--------|---------|-------------|
| `onboarding` | unset | show first-run notification setup; set `false` after choosing |

notes:
- missing `onboarding` currently behaves like `true`
- set `onboarding = true` to force the setup screen again for testing
- after onboarding, herdr writes `onboarding = false` plus the chosen sound/toast settings

## keybindings

keybindings live under `[keys]`.

supported syntax:
- plain keys: `n`, `x`, `-`, `` ` ``
- modifiers: `ctrl+b`, `shift+n`, `alt+x`
- special keys: `enter`, `esc`, `tab`, `backspace`
- function keys: `f1`, `f12`
- uppercase letters also imply shift: `D` works like `shift+d`

notes:
- most reliable bindings are plain keys, `ctrl+letter`, `esc`/`tab`/`enter`, and function keys
- `alt+...` and punctuation-with-modifiers may vary depending on terminal/tmux setup
- for navigate-mode actions, duplicate keybindings are treated as config errors; later conflicting bindings fall back to defaults

example:

```toml
[keys]
prefix = "ctrl+b"
new_workspace = "n"
rename_workspace = "shift+n"
close_workspace = "d"
split_vertical = "v"
split_horizontal = "-"
close_pane = "x"
fullscreen = "f"
resize_mode = "r"
toggle_sidebar = "b"
```

### key reference

| key | default | action |
|-----|---------|--------|
| `prefix` | `ctrl+b` | enter or leave navigate mode |
| `new_workspace` | `n` | create a new workspace |
| `rename_workspace` | `shift+n` | rename selected workspace |
| `close_workspace` | `d` | close selected workspace |
| `split_vertical` | `v` | split pane vertically (side by side) |
| `split_horizontal` | `-` | split pane horizontally (stacked) |
| `close_pane` | `x` | close focused pane |
| `fullscreen` | `f` | toggle focused pane fullscreen |
| `resize_mode` | `r` | enter or leave resize mode |
| `toggle_sidebar` | `b` | collapse or expand the sidebar |

## ui

```toml
[ui]
sidebar_width = 26
confirm_close = true
accent = "cyan"
```

### options

| option | default | description |
|--------|---------|-------------|
| `sidebar_width` | `26` | base sidebar width before auto-scaling |
| `confirm_close` | `true` | ask before closing a workspace |
| `accent` | `cyan` | highlight and border color |

`accent` accepts:
- named colors like `cyan`, `blue`, `magenta`
- hex like `#89b4fa`
- rgb like `rgb(137,180,250)`

## toast notifications

```toml
[ui.toast]
enabled = false
```

### options

| option | default | description |
|--------|---------|-------------|
| `ui.toast.enabled` | `false` | show top-right visual toasts for background agent events |

current v1 behavior:
- informational only
- one toast at a time
- top-right placement
- shown for background agent events like `needs attention` and `finished`
- no keyboard action or temporary key semantics

## sound

```toml
[ui.sound]
enabled = true

[ui.sound.agents]
claude = "default"
droid = "off"
```

### options

| option | default | description |
|--------|---------|-------------|
| `ui.sound.enabled` | `true` | enable background agent sounds |

per-agent values:
- `default`
- `on`
- `off`

available agent keys:
- `pi`
- `claude`
- `codex`
- `gemini`
- `cursor`
- `cline`
- `open_code`
- `github_copilot`
- `kimi`
- `droid`
- `amp`

## environment variables

| variable | description |
|----------|-------------|
| `HERDR_LOG` | log level filter (default: `herdr=info`) |

logs are written to:

```text
~/.config/herdr/herdr.log
```
