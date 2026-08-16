# niri-scratch

Fast, stateful scratchpad workspaces for [Niri](https://github.com/YaLTeR/niri), written in Rust.
Press a binding once to open a dedicated workspace and press it again to return to the exact
workspace/window you came from. The daemon talks directly to Niri IPC, serializes toggles to avoid
races, adopts matching windows when configured, and never depends on `jq` or shell scripts in the
hot path.

> Niri has no hidden “special workspace” primitive. `niri-scratch` implements scratchpads with
> named regular workspaces such as `scratch:terminal`; they can therefore still appear in Overview.

## CachyOS / Arch Linux: install from scratch

Install Niri and the build/runtime dependencies. Applications are examples and may be replaced in
the TOML configuration.

```bash
sudo pacman -Syu
sudo pacman -S --needed git rustup niri kitty firefox telegram-desktop
rustup default stable

git clone https://github.com/mickberrad659-sketch/niri-scratch.git
cd niri-scratch
cargo build --release --locked

install -Dm755 target/release/niri-scratch ~/.local/bin/niri-scratch
install -Dm644 config/example.toml ~/.config/niri-scratch/config.toml
install -Dm644 contrib/niri-scratch.service ~/.config/systemd/user/niri-scratch.service
mkdir -p ~/.local/state/niri-scratch
chmod 700 ~/.local/state/niri-scratch
systemctl --user daemon-reload
systemctl --user enable niri-scratch.service
```

Add the contents of [`contrib/niri-bindings.kdl`](contrib/niri-bindings.kdl) to your Niri config.
Also add this ordered startup command at top level in `~/.config/niri/config.kdl`:

```kdl
spawn-sh-at-startup "systemctl --user import-environment NIRI_SOCKET WAYLAND_DISPLAY && systemctl --user restart niri-scratch.service"
```

Do not start the service from Plasma or a TTY: it intentionally requires `NIRI_SOCKET`. Log into
Niri normally after installation, then run `niri-scratch doctor`.

## Install into an existing Niri setup

The procedure is the same, but merge the KDL snippets instead of replacing your config:

1. Build and install the binary, example config, and user service using the commands above.
2. Add the three top-level `workspace "scratch:…"` declarations from
   [`contrib/niri-bindings.kdl`](contrib/niri-bindings.kdl).
3. Copy its bindings into your existing `binds {}` block and resolve key conflicts.
4. Add the ordered `spawn-sh-at-startup` command shown above.
5. Check everything before ending the current session:

```bash
niri validate
niri-scratch check-config
systemd-analyze --user verify ~/.config/systemd/user/niri-scratch.service
```

The example bindings are:

| Binding | Scratchpad |
|---|---|
| `Mod+S` | Firefox in `scratch:web` |
| `Mod+D` | Empty `scratch:d` workspace |
| `Mod+F` | Empty `scratch:f` workspace |
| `Mod+/` | Telegram and Throne in `scratch:chat` |
| `Mod+U` | Empty `scratch:u` workspace |

Edit `~/.config/niri-scratch/config.toml` if an application's command or `app_id` differs. Discover
actual IDs inside Niri with `niri msg windows`.

## Optional Noctalia widget

Copy the local plugin and enable it in Noctalia's plugin registry/UI:

```bash
cp -a contrib/noctalia/niri-workspaces ~/.config/noctalia/plugins/
```

Replace the generic `Workspace` entry with `{ "id": "plugin:niri-workspaces" }` under
`bar.widgets` in `~/.config/noctalia/settings.json`. Add an enabled `niri-workspaces` state in
`~/.config/noctalia/plugins.json`, or enable them through Noctalia when local plugins are shown.
The `niri-workspaces` widget deliberately shows only named workspaces 1–6. While a scratchpad is
active, its icon replaces the number of the return workspace, and the capsule changes color. This
keeps scratchpads out of the normal workspace list without adding another bar group.

For multi-application scratchpads, `commands` launches every configured command once. Optional
`initial_focus_app_id` selects the window that receives focus after all windows are anchored;
`initial_focus_full_width = true` moves its column first and expands it to the available width.
Later visits leave focus selection to Niri, preserving the scratchpad's last-focused application.

## Commands

```bash
niri-scratch toggle terminal
niri-scratch show web
niri-scratch hide telegram
niri-scratch hide-all
niri-scratch status
niri-scratch --json status
niri-scratch doctor
niri-scratch check-config
```

For daemon diagnostics:

```bash
systemctl --user status niri-scratch.service
journalctl --user -u niri-scratch.service -b
```

## Development

```bash
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release --locked
```

The integration test starts the real daemon against a fake Niri Unix socket and checks a complete
toggle round trip. The design and reliability plan is documented in [`PLAN.md`](PLAN.md).

License: GPL-3.0-or-later.
