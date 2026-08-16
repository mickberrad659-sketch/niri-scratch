use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use niri_ipc::socket::Socket as NiriSocket;
use niri_ipc::{
    Action, Request as NiriRequest, Response, SizeChange, Window, Workspace, WorkspaceReferenceArg,
};
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub scratchpads: BTreeMap<String, ScratchpadConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DaemonConfig {
    pub socket: Option<PathBuf>,
    pub state_file: Option<PathBuf>,
    pub launch_timeout_ms: u64,
    pub background_anchor: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ScratchpadConfig {
    pub workspace: String,
    pub command: Vec<String>,
    pub commands: Vec<Vec<String>>,
    pub match_app_id: Option<String>,
    pub match_title: Option<String>,
    pub launch_if_missing: bool,
    pub adopt_existing: bool,
    pub initial_focus_app_id: Option<String>,
    pub initial_focus_full_width: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket: None,
            state_file: None,
            launch_timeout_ms: 8_000,
            background_anchor: true,
        }
    }
}

impl Default for ScratchpadConfig {
    fn default() -> Self {
        Self {
            workspace: String::new(),
            command: Vec::new(),
            commands: Vec::new(),
            match_app_id: None,
            match_title: None,
            launch_if_missing: true,
            adopt_existing: true,
            initial_focus_app_id: None,
            initial_focus_full_width: false,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let data = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let config: Self =
            toml::from_str(&data).with_context(|| format!("invalid TOML in {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.scratchpads.is_empty() {
            bail!("at least one scratchpad must be configured");
        }
        let mut workspaces = BTreeMap::<&str, &str>::new();
        for (name, pad) in &self.scratchpads {
            if name.trim().is_empty() || name.chars().any(char::is_whitespace) {
                bail!("scratchpad name must be non-empty and contain no whitespace: {name:?}");
            }
            if pad.workspace.trim().is_empty() {
                bail!("scratchpad {name}: workspace must not be empty");
            }
            if matches!(pad.workspace.as_str(), "1" | "2" | "3" | "4" | "5" | "6") {
                bail!("scratchpad {name}: workspace conflicts with normal workspace 1-6");
            }
            if let Some(previous) = workspaces.insert(&pad.workspace, name) {
                bail!(
                    "scratchpads {previous} and {name} share workspace {}",
                    pad.workspace
                );
            }
            if pad.command.is_empty() && pad.commands.is_empty() && pad.launch_if_missing {
                bail!(
                    "scratchpad {name}: command or commands is required when launch_if_missing=true"
                );
            }
            if pad.commands.iter().any(Vec::is_empty) {
                bail!("scratchpad {name}: commands must not contain an empty command");
            }
            if let Some(pattern) = &pad.match_app_id {
                Regex::new(pattern)
                    .with_context(|| format!("scratchpad {name}: invalid app_id regex"))?;
            }
            if let Some(pattern) = &pad.match_title {
                Regex::new(pattern)
                    .with_context(|| format!("scratchpad {name}: invalid title regex"))?;
            }
            if let Some(pattern) = &pad.initial_focus_app_id {
                Regex::new(pattern)
                    .with_context(|| format!("scratchpad {name}: invalid initial focus regex"))?;
            }
        }
        Ok(())
    }

    pub fn socket_path(&self) -> Result<PathBuf> {
        if let Some(path) = &self.daemon.socket {
            return Ok(expand_path(path));
        }
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("XDG_RUNTIME_DIR is not set"))?;
        Ok(runtime.join("niri-scratch.sock"))
    }

    pub fn state_path(&self) -> Result<PathBuf> {
        if let Some(path) = &self.daemon.state_file {
            return Ok(expand_path(path));
        }
        let root = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))
            .ok_or_else(|| anyhow!("neither XDG_STATE_HOME nor HOME is set"))?;
        Ok(root.join("niri-scratch/state.json"))
    }
}

fn expand_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let home = env::var("HOME").unwrap_or_default();
    let runtime = env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    let state = env::var("XDG_STATE_HOME").unwrap_or_else(|_| format!("{home}/.local/state"));
    PathBuf::from(
        text.replace("${HOME}", &home)
            .replace("${XDG_RUNTIME_DIR}", &runtime)
            .replace("${XDG_STATE_HOME}", &state),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReturnTarget {
    pub workspace_id: u64,
    pub workspace_name: Option<String>,
    pub output_name: Option<String>,
    pub focused_window_id: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    pub origins: BTreeMap<String, ReturnTarget>,
}

impl PersistedState {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("state path has no parent"))?;
        fs::create_dir_all(parent)?;
        let temp = path.with_extension("json.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(temp, path)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum ControlRequest {
    Toggle { scratchpad: String },
    Show { scratchpad: String },
    Hide { scratchpad: String },
    HideAll,
    Status { scratchpad: Option<String> },
    List,
    Doctor,
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlResponse {
    pub version: u8,
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ControlResponse {
    fn ok(message: impl Into<String>, data: Option<serde_json::Value>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            ok: true,
            message: message.into(),
            data,
        }
    }

    fn error(error: impl std::fmt::Display) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            ok: false,
            message: error.to_string(),
            data: None,
        }
    }
}

pub trait Compositor {
    fn workspaces(&mut self) -> Result<Vec<Workspace>>;
    fn windows(&mut self) -> Result<Vec<Window>>;
    fn action(&mut self, action: Action) -> Result<()>;
    fn version(&mut self) -> Result<String>;
}

pub struct NiriCompositor;

impl NiriCompositor {
    fn request(request: NiriRequest) -> Result<Response> {
        let mut socket = NiriSocket::connect().context("cannot connect to NIRI_SOCKET")?;
        socket
            .send(request)
            .context("Niri IPC transport error")?
            .map_err(|message| anyhow!("Niri rejected request: {message}"))
    }
}

impl Compositor for NiriCompositor {
    fn workspaces(&mut self) -> Result<Vec<Workspace>> {
        match Self::request(NiriRequest::Workspaces)? {
            Response::Workspaces(value) => Ok(value),
            other => bail!("unexpected workspaces response: {other:?}"),
        }
    }

    fn windows(&mut self) -> Result<Vec<Window>> {
        match Self::request(NiriRequest::Windows)? {
            Response::Windows(value) => Ok(value),
            other => bail!("unexpected windows response: {other:?}"),
        }
    }

    fn action(&mut self, action: Action) -> Result<()> {
        match Self::request(NiriRequest::Action(action))? {
            Response::Handled => Ok(()),
            other => bail!("unexpected action response: {other:?}"),
        }
    }

    fn version(&mut self) -> Result<String> {
        match Self::request(NiriRequest::Version)? {
            Response::Version(value) => Ok(value),
            other => bail!("unexpected version response: {other:?}"),
        }
    }
}

pub struct Engine<C: Compositor> {
    pub config: Config,
    pub state: PersistedState,
    pub compositor: C,
    state_path: PathBuf,
    pending_until: BTreeMap<String, Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Desired {
    Toggle,
    Show,
    Hide,
}

impl<C: Compositor> Engine<C> {
    pub fn new(config: Config, compositor: C) -> Result<Self> {
        let state_path = config.state_path()?;
        let state = PersistedState::load(&state_path);
        Ok(Self {
            config,
            state,
            compositor,
            state_path,
            pending_until: BTreeMap::new(),
        })
    }

    pub fn handle(&mut self, request: ControlRequest) -> Result<ControlResponse> {
        match request {
            ControlRequest::Toggle { scratchpad } => self.transition(&scratchpad, Desired::Toggle),
            ControlRequest::Show { scratchpad } => self.transition(&scratchpad, Desired::Show),
            ControlRequest::Hide { scratchpad } => self.transition(&scratchpad, Desired::Hide),
            ControlRequest::HideAll => self.hide_all(),
            ControlRequest::Status { scratchpad } => self.status(scratchpad.as_deref()),
            ControlRequest::List => Ok(ControlResponse::ok(
                "configured scratchpads",
                Some(serde_json::to_value(&self.config.scratchpads)?),
            )),
            ControlRequest::Doctor => self.doctor(),
            ControlRequest::Ping => Ok(ControlResponse::ok("pong", None)),
        }
    }

    fn snapshot(&mut self) -> Result<(Vec<Workspace>, Vec<Window>)> {
        let workspaces = self.compositor.workspaces()?;
        let windows = self.compositor.windows()?;
        Ok((workspaces, windows))
    }

    fn transition(&mut self, name: &str, desired: Desired) -> Result<ControlResponse> {
        let pad = self
            .config
            .scratchpads
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("unknown scratchpad {name:?}"))?;
        let (workspaces, windows) = self.snapshot()?;
        let focused = workspaces
            .iter()
            .find(|workspace| workspace.is_focused)
            .ok_or_else(|| anyhow!("Niri reports no focused workspace"))?;
        let is_visible = focused.name.as_deref() == Some(pad.workspace.as_str());
        let show = match desired {
            Desired::Toggle => !is_visible,
            Desired::Show => true,
            Desired::Hide => false,
        };

        if show {
            self.show(name, &pad, focused, &workspaces, &windows)
        } else if is_visible {
            self.hide(name, &workspaces, &windows)
        } else {
            Ok(ControlResponse::ok(
                format!("{name} is already hidden"),
                None,
            ))
        }
    }

    fn show(
        &mut self,
        name: &str,
        pad: &ScratchpadConfig,
        focused: &Workspace,
        workspaces: &[Workspace],
        windows: &[Window],
    ) -> Result<ControlResponse> {
        if focused.name.as_deref() != Some(pad.workspace.as_str()) {
            let direct_origin = ReturnTarget {
                workspace_id: focused.id,
                workspace_name: focused.name.clone(),
                output_name: focused.output.clone(),
                focused_window_id: focused.active_window_id,
            };
            // Opening one scratchpad from another should still return to the last normal
            // workspace. Flatten the origin chain instead of creating nested scratchpads.
            let origin = self
                .config
                .scratchpads
                .iter()
                .find(|(_, other)| focused.name.as_ref() == Some(&other.workspace))
                .and_then(|(other_name, _)| self.state.origins.get(other_name).cloned())
                .unwrap_or(direct_origin);
            self.state.origins.insert(name.to_string(), origin);
            self.state.save(&self.state_path)?;
        }

        let scratch_workspace = workspaces
            .iter()
            .find(|workspace| workspace.name.as_deref() == Some(pad.workspace.as_str()));
        let scratch_workspace_id = scratch_workspace.map(|workspace| workspace.id);
        let matched = matching_windows(pad, windows)?;
        let selected: Vec<_> = matched
            .iter()
            .copied()
            .filter(|window| window.workspace_id == scratch_workspace_id || pad.adopt_existing)
            .collect();

        for window in &selected {
            if pad.adopt_existing && window.workspace_id != scratch_workspace_id {
                self.compositor.action(Action::MoveWindowToWorkspace {
                    window_id: Some(window.id),
                    reference: WorkspaceReferenceArg::Name(pad.workspace.clone()),
                    focus: false,
                })?;
            }
        }

        self.compositor.action(Action::FocusWorkspace {
            reference: WorkspaceReferenceArg::Name(pad.workspace.clone()),
        })?;

        if !selected.is_empty() {
            // Ask Niri for the workspace's own last-focused window. This is dynamic state, not
            // "the first matching app", so Telegram/Throne resume exactly where the user left.
            if let Some(last_window_id) = scratch_workspace.and_then(|ws| ws.active_window_id)
                && selected.iter().any(|window| window.id == last_window_id)
            {
                self.compositor
                    .action(Action::FocusWindow { id: last_window_id })?;
            }
            return Ok(ControlResponse::ok(format!("shown {name}"), None));
        }

        if pad.launch_if_missing {
            if self
                .pending_until
                .get(name)
                .is_some_and(|deadline| *deadline > Instant::now())
            {
                return Ok(ControlResponse::ok(
                    format!("shown {name}; application launch already pending"),
                    None,
                ));
            }
            let windows_before: HashSet<u64> = windows.iter().map(|window| window.id).collect();
            let launch_commands: Vec<Vec<String>> = if pad.commands.is_empty() {
                vec![pad.command.clone()]
            } else {
                pad.commands.clone()
            };
            for command in &launch_commands {
                self.compositor.action(Action::Spawn {
                    command: command.clone(),
                })?;
            }

            let timeout = Duration::from_millis(self.config.daemon.launch_timeout_ms);
            self.pending_until
                .insert(name.to_string(), Instant::now() + timeout);

            if self.config.daemon.background_anchor && !timeout.is_zero() {
                spawn_anchor_worker(pad.clone(), windows_before, launch_commands.len(), timeout);
                return Ok(ControlResponse::ok(
                    format!("shown {name}; application launch scheduled"),
                    None,
                ));
            }

            // Niri's activation token normally places a spawned window on the workspace where the
            // action originated. Keep an IPC-side safety net for slow applications: if the user
            // changes workspace before the window maps, anchor the newly created matching window
            // back to this scratchpad without stealing focus.
            let deadline = Instant::now() + timeout;
            let mut anchored = HashSet::new();
            while Instant::now() < deadline {
                thread::sleep(Duration::from_millis(50));
                let current_windows = self.compositor.windows()?;
                let new_window_ids: Vec<u64> = matching_windows(pad, &current_windows)?
                    .into_iter()
                    .map(|window| window.id)
                    .filter(|id| !windows_before.contains(id) && !anchored.contains(id))
                    .collect();
                for window_id in new_window_ids {
                    self.compositor.action(Action::MoveWindowToWorkspace {
                        window_id: Some(window_id),
                        reference: WorkspaceReferenceArg::Name(pad.workspace.clone()),
                        focus: false,
                    })?;
                    anchored.insert(window_id);
                }
                if anchored.len() >= launch_commands.len() {
                    if let Some(pattern) = &pad.initial_focus_app_id {
                        let pattern = Regex::new(pattern)?;
                        if let Some(window) = current_windows.iter().find(|window| {
                            anchored.contains(&window.id)
                                && window
                                    .app_id
                                    .as_deref()
                                    .is_some_and(|app_id| pattern.is_match(app_id))
                        }) {
                            self.compositor
                                .action(Action::FocusWindow { id: window.id })?;
                            if pad.initial_focus_full_width {
                                self.compositor.action(Action::MoveColumnToFirst {})?;
                                self.compositor.action(Action::SetColumnWidth {
                                    change: SizeChange::SetProportion(1.0),
                                })?;
                            }
                        }
                    }
                    return Ok(ControlResponse::ok(
                        format!("shown {name}; application launched and anchored"),
                        None,
                    ));
                }
            }
            return Ok(ControlResponse::ok(
                format!("shown {name}; application launched"),
                None,
            ));
        }

        Ok(ControlResponse::ok(
            format!("shown empty scratchpad {name}"),
            None,
        ))
    }

    fn hide(
        &mut self,
        name: &str,
        workspaces: &[Workspace],
        windows: &[Window],
    ) -> Result<ControlResponse> {
        let origin = self.state.origins.remove(name);
        self.state.save(&self.state_path)?;
        if let Some(origin) = origin {
            let target = workspaces
                .iter()
                .find(|workspace| workspace.id == origin.workspace_id)
                .or_else(|| {
                    origin.workspace_name.as_ref().and_then(|wanted| {
                        workspaces.iter().find(|workspace| {
                            workspace.name.as_ref() == Some(wanted)
                                && (origin.output_name.is_none()
                                    || workspace.output == origin.output_name)
                        })
                    })
                });
            if let Some(target) = target {
                self.compositor.action(Action::FocusWorkspace {
                    reference: WorkspaceReferenceArg::Id(target.id),
                })?;
                if let Some(window_id) = origin.focused_window_id
                    && windows.iter().any(|window| {
                        window.id == window_id && window.workspace_id == Some(target.id)
                    })
                {
                    self.compositor
                        .action(Action::FocusWindow { id: window_id })?;
                }
                return Ok(ControlResponse::ok(
                    format!("hidden {name}; origin restored"),
                    None,
                ));
            }
        }
        self.compositor.action(Action::FocusWorkspacePrevious {})?;
        Ok(ControlResponse::ok(
            format!("hidden {name}; used previous workspace fallback"),
            None,
        ))
    }

    fn hide_all(&mut self) -> Result<ControlResponse> {
        let workspaces = self.compositor.workspaces()?;
        let focused_name = workspaces
            .iter()
            .find(|workspace| workspace.is_focused)
            .and_then(|workspace| workspace.name.clone());
        if let Some((name, _)) = self
            .config
            .scratchpads
            .iter()
            .find(|(_, pad)| Some(&pad.workspace) == focused_name.as_ref())
            .map(|(name, pad)| (name.clone(), pad.clone()))
        {
            let windows = self.compositor.windows()?;
            return self.hide(&name, &workspaces, &windows);
        }
        Ok(ControlResponse::ok("no scratchpad is focused", None))
    }

    fn status(&mut self, only: Option<&str>) -> Result<ControlResponse> {
        let (workspaces, windows) = self.snapshot()?;
        let mut values = BTreeMap::new();
        for (name, pad) in &self.config.scratchpads {
            if only.is_some_and(|wanted| wanted != name) {
                continue;
            }
            let workspace = workspaces
                .iter()
                .find(|workspace| workspace.name.as_deref() == Some(&pad.workspace));
            let matching = matching_windows(pad, &windows)?;
            values.insert(
                name.clone(),
                serde_json::json!({
                    "workspace": pad.workspace,
                    "visible": workspace.is_some_and(|workspace| workspace.is_focused),
                    "active": workspace.is_some_and(|workspace| workspace.is_active),
                    "window_ids": matching.iter().map(|window| window.id).collect::<Vec<_>>(),
                    "origin": self.state.origins.get(name),
                }),
            );
        }
        if let Some(name) = only
            && !self.config.scratchpads.contains_key(name)
        {
            bail!("unknown scratchpad {name:?}");
        }
        Ok(ControlResponse::ok(
            "status",
            Some(serde_json::to_value(values)?),
        ))
    }

    fn doctor(&mut self) -> Result<ControlResponse> {
        self.config.validate()?;
        let version = self.compositor.version()?;
        let workspaces = self.compositor.workspaces()?;
        let missing_workspaces: Vec<_> = self
            .config
            .scratchpads
            .iter()
            .filter(|(_, pad)| {
                !workspaces
                    .iter()
                    .any(|workspace| workspace.name.as_deref() == Some(&pad.workspace))
            })
            .map(|(name, pad)| serde_json::json!({"scratchpad": name, "workspace": pad.workspace}))
            .collect();
        Ok(ControlResponse::ok(
            "doctor completed",
            Some(serde_json::json!({
                "niri_version": version,
                "socket": env::var("NIRI_SOCKET").ok(),
                "missing_workspaces": missing_workspaces,
                "healthy": missing_workspaces.is_empty(),
            })),
        ))
    }
}

fn spawn_anchor_worker(
    pad: ScratchpadConfig,
    windows_before: HashSet<u64>,
    expected_windows: usize,
    timeout: Duration,
) {
    thread::spawn(move || {
        if let Err(error) = anchor_spawned_windows(&pad, &windows_before, expected_windows, timeout)
        {
            eprintln!("background anchor for {} failed: {error:#}", pad.workspace);
        }
    });
}

fn anchor_spawned_windows(
    pad: &ScratchpadConfig,
    windows_before: &HashSet<u64>,
    expected_windows: usize,
    timeout: Duration,
) -> Result<()> {
    let mut compositor = NiriCompositor;
    let deadline = Instant::now() + timeout;
    let mut anchored = HashSet::new();
    let mut latest_windows = Vec::new();

    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
        latest_windows = compositor.windows()?;
        let new_window_ids: Vec<u64> = matching_windows(pad, &latest_windows)?
            .into_iter()
            .map(|window| window.id)
            .filter(|id| !windows_before.contains(id) && !anchored.contains(id))
            .collect();
        for window_id in new_window_ids {
            compositor.action(Action::MoveWindowToWorkspace {
                window_id: Some(window_id),
                reference: WorkspaceReferenceArg::Name(pad.workspace.clone()),
                focus: false,
            })?;
            anchored.insert(window_id);
        }
        if anchored.len() >= expected_windows {
            break;
        }
    }

    if anchored.len() < expected_windows {
        return Ok(());
    }

    // Initial focus/layout is allowed only while the user is still looking at this scratchpad.
    // Leaving during a slow launch must never teleport them back.
    let still_visible = compositor.workspaces()?.iter().any(|workspace| {
        workspace.is_focused && workspace.name.as_deref() == Some(pad.workspace.as_str())
    });
    if still_visible && let Some(pattern) = &pad.initial_focus_app_id {
        let pattern = Regex::new(pattern)?;
        if let Some(window) = latest_windows.iter().find(|window| {
            anchored.contains(&window.id)
                && window
                    .app_id
                    .as_deref()
                    .is_some_and(|app_id| pattern.is_match(app_id))
        }) {
            compositor.action(Action::FocusWindow { id: window.id })?;
            if pad.initial_focus_full_width {
                compositor.action(Action::MoveColumnToFirst {})?;
                compositor.action(Action::SetColumnWidth {
                    change: SizeChange::SetProportion(1.0),
                })?;
            }
        }
    }
    Ok(())
}

fn matching_windows<'a>(pad: &ScratchpadConfig, windows: &'a [Window]) -> Result<Vec<&'a Window>> {
    let app = pad.match_app_id.as_deref().map(Regex::new).transpose()?;
    let title = pad.match_title.as_deref().map(Regex::new).transpose()?;
    Ok(windows
        .iter()
        .filter(|window| {
            let app_matches = app.as_ref().is_none_or(|pattern| {
                window
                    .app_id
                    .as_deref()
                    .is_some_and(|value| pattern.is_match(value))
            });
            let title_matches = title.as_ref().is_none_or(|pattern| {
                window
                    .title
                    .as_deref()
                    .is_some_and(|value| pattern.is_match(value))
            });
            (app.is_some() || title.is_some()) && app_matches && title_matches
        })
        .collect())
}

pub fn run_daemon(config: Config) -> Result<()> {
    let socket_path = config.socket_path()?;
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        if UnixStream::connect(&socket_path).is_ok() {
            bail!("daemon is already running at {}", socket_path.display());
        }
        fs::remove_file(&socket_path)
            .with_context(|| format!("cannot remove stale socket {}", socket_path.display()))?;
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("cannot bind {}", socket_path.display()))?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    let mut engine = Engine::new(config, NiriCompositor)?;
    eprintln!("niri-scratch daemon listening on {}", socket_path.display());

    for incoming in listener.incoming() {
        let mut stream = match incoming {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("control socket accept error: {error}");
                continue;
            }
        };
        let response =
            match read_control_request(&stream).and_then(|request| engine.handle(request)) {
                Ok(response) => response,
                Err(error) => ControlResponse::error(error),
            };
        serde_json::to_writer(&mut stream, &response)?;
        stream.write_all(b"\n")?;
    }
    Ok(())
}

fn read_control_request(stream: &UnixStream) -> Result<ControlRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut line = String::new();
    BufReader::new(stream)
        .take(64 * 1024)
        .read_line(&mut line)?;
    if line.is_empty() {
        bail!("empty control request");
    }
    serde_json::from_str(&line).context("invalid control request")
}

pub fn send_control(config: &Config, request: &ControlRequest) -> Result<ControlResponse> {
    let socket_path = config.socket_path()?;
    let mut stream = UnixStream::connect(&socket_path).with_context(|| {
        format!(
            "cannot connect to daemon at {}; is niri-scratch.service running?",
            socket_path.display()
        )
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    serde_json::to_writer(&mut stream, request)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut line = String::new();
    BufReader::new(stream)
        .take(1024 * 1024)
        .read_line(&mut line)?;
    serde_json::from_str(&line).context("invalid daemon response")
}

pub fn default_config_path() -> Result<PathBuf> {
    let root = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or_else(|| anyhow!("neither XDG_CONFIG_HOME nor HOME is set"))?;
    Ok(root.join("niri-scratch/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use niri_ipc::{Timestamp, WindowLayout};

    #[derive(Default)]
    struct FakeCompositor {
        workspaces: Vec<Workspace>,
        windows: Vec<Window>,
        actions: Vec<Action>,
    }

    impl Compositor for FakeCompositor {
        fn workspaces(&mut self) -> Result<Vec<Workspace>> {
            Ok(self.workspaces.clone())
        }
        fn windows(&mut self) -> Result<Vec<Window>> {
            Ok(self.windows.clone())
        }
        fn action(&mut self, action: Action) -> Result<()> {
            self.actions.push(action);
            Ok(())
        }
        fn version(&mut self) -> Result<String> {
            Ok("26.04".into())
        }
    }

    struct DelayedWindowCompositor {
        workspaces: Vec<Workspace>,
        window_calls: usize,
        actions: Vec<Action>,
    }

    impl Compositor for DelayedWindowCompositor {
        fn workspaces(&mut self) -> Result<Vec<Workspace>> {
            Ok(self.workspaces.clone())
        }
        fn windows(&mut self) -> Result<Vec<Window>> {
            self.window_calls += 1;
            if self.window_calls == 1 {
                Ok(vec![])
            } else {
                // Simulate Firefox mapping on the workspace selected by a very fast user switch.
                Ok(vec![
                    window(42, "org.telegram.desktop", 2),
                    window(43, "Throne", 2),
                ])
            }
        }
        fn action(&mut self, action: Action) -> Result<()> {
            self.actions.push(action);
            Ok(())
        }
        fn version(&mut self) -> Result<String> {
            Ok("26.04".into())
        }
    }

    fn workspace(id: u64, name: &str, focused: bool) -> Workspace {
        Workspace {
            id,
            idx: id as u8,
            name: Some(name.into()),
            output: Some("eDP-1".into()),
            is_urgent: false,
            is_active: focused,
            is_focused: focused,
            active_window_id: None,
        }
    }

    fn window(id: u64, app_id: &str, workspace_id: u64) -> Window {
        Window {
            id,
            title: Some("test".into()),
            app_id: Some(app_id.into()),
            pid: Some(123),
            workspace_id: Some(workspace_id),
            is_focused: false,
            is_floating: false,
            is_urgent: false,
            layout: WindowLayout {
                pos_in_scrolling_layout: Some((1, 1)),
                tile_size: (100.0, 100.0),
                window_size: (100, 100),
                tile_pos_in_workspace_view: Some((0.0, 0.0)),
                window_offset_in_tile: (0.0, 0.0),
            },
            focus_timestamp: Some(Timestamp { secs: 1, nanos: 0 }),
        }
    }

    fn test_config(state_path: &Path) -> Config {
        Config {
            daemon: DaemonConfig {
                socket: None,
                state_file: Some(state_path.into()),
                launch_timeout_ms: 10,
                background_anchor: false,
            },
            scratchpads: BTreeMap::from([(
                "terminal".into(),
                ScratchpadConfig {
                    workspace: "scratch:terminal".into(),
                    command: vec!["kitty".into()],
                    commands: vec![],
                    match_app_id: Some("^scratch-terminal$".into()),
                    match_title: None,
                    launch_if_missing: true,
                    adopt_existing: true,
                    initial_focus_app_id: None,
                    initial_focus_full_width: false,
                },
            )]),
        }
    }

    #[test]
    fn config_rejects_normal_workspace_collision() {
        let mut config = test_config(Path::new("/tmp/test-state"));
        config.scratchpads.get_mut("terminal").unwrap().workspace = "3".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_accepts_empty_scratchpad_when_launch_is_disabled() {
        let mut config = test_config(Path::new("/tmp/test-state"));
        let pad = config.scratchpads.get_mut("terminal").unwrap();
        pad.command.clear();
        pad.match_app_id = None;
        pad.launch_if_missing = false;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn newly_spawned_windows_are_anchored_after_fast_workspace_switch() {
        let dir = tempfile::tempdir().unwrap();
        let compositor = DelayedWindowCompositor {
            workspaces: vec![
                workspace(1, "1", true),
                workspace(2, "2", false),
                workspace(20, "scratch:terminal", false),
            ],
            window_calls: 0,
            actions: vec![],
        };
        let mut config = test_config(&dir.path().join("state.json"));
        config.daemon.launch_timeout_ms = 200;
        let pad = config.scratchpads.get_mut("terminal").unwrap();
        pad.adopt_existing = false;
        pad.command.clear();
        pad.commands = vec![vec!["Telegram".into()], vec!["throne".into()]];
        pad.match_app_id = Some("^(org\\.telegram\\.desktop|Throne)$".into());
        pad.initial_focus_app_id = Some("^org\\.telegram\\.desktop$".into());
        pad.initial_focus_full_width = true;
        let mut engine = Engine::new(config, compositor).unwrap();

        let response = engine
            .handle(ControlRequest::Show {
                scratchpad: "terminal".into(),
            })
            .unwrap();

        assert!(response.message.contains("launched and anchored"));
        assert!(engine.compositor.actions.iter().any(|action| matches!(
            action,
            Action::MoveWindowToWorkspace {
                window_id: Some(42),
                reference: WorkspaceReferenceArg::Name(name),
                focus: false,
            } if name == "scratch:terminal"
        )));
        assert!(engine.compositor.actions.iter().any(|action| matches!(
            action,
            Action::MoveWindowToWorkspace {
                window_id: Some(43),
                reference: WorkspaceReferenceArg::Name(name),
                focus: false,
            } if name == "scratch:terminal"
        )));
        let focus_position = engine
            .compositor
            .actions
            .iter()
            .position(|action| matches!(action, Action::FocusWindow { id: 42 }))
            .unwrap();
        assert!(matches!(
            engine.compositor.actions[focus_position + 1],
            Action::MoveColumnToFirst {}
        ));
        assert!(matches!(
            engine.compositor.actions[focus_position + 2],
            Action::SetColumnWidth {
                change: SizeChange::SetProportion(1.0)
            }
        ));
    }

    #[test]
    fn show_remembers_origin_without_overriding_last_scratch_focus() {
        let dir = tempfile::tempdir().unwrap();
        let compositor = FakeCompositor {
            workspaces: vec![
                workspace(1, "1", true),
                workspace(20, "scratch:terminal", false),
            ],
            windows: vec![window(9, "scratch-terminal", 20)],
            actions: vec![],
        };
        let mut engine =
            Engine::new(test_config(&dir.path().join("state.json")), compositor).unwrap();
        let response = engine
            .handle(ControlRequest::Toggle {
                scratchpad: "terminal".into(),
            })
            .unwrap();
        assert!(response.ok);
        assert_eq!(engine.state.origins["terminal"].workspace_id, 1);
        assert!(matches!(
            engine.compositor.actions[0],
            Action::FocusWorkspace { .. }
        ));
        assert_eq!(engine.compositor.actions.len(), 1);
    }

    #[test]
    fn show_restores_workspace_active_window_instead_of_first_match() {
        let dir = tempfile::tempdir().unwrap();
        let mut scratch = workspace(20, "scratch:terminal", false);
        scratch.active_window_id = Some(10);
        let compositor = FakeCompositor {
            workspaces: vec![workspace(1, "1", true), scratch],
            windows: vec![
                window(9, "scratch-terminal", 20),
                window(10, "scratch-terminal", 20),
            ],
            actions: vec![],
        };
        let mut engine =
            Engine::new(test_config(&dir.path().join("state.json")), compositor).unwrap();
        engine
            .handle(ControlRequest::Show {
                scratchpad: "terminal".into(),
            })
            .unwrap();
        assert!(matches!(
            engine.compositor.actions.as_slice(),
            [
                Action::FocusWorkspace { .. },
                Action::FocusWindow { id: 10 }
            ]
        ));
    }

    #[test]
    fn pending_launch_suppresses_duplicate_spawn_during_rapid_toggle() {
        let dir = tempfile::tempdir().unwrap();
        let compositor = FakeCompositor {
            workspaces: vec![
                workspace(1, "1", true),
                workspace(20, "scratch:terminal", false),
            ],
            windows: vec![],
            actions: vec![],
        };
        let mut engine =
            Engine::new(test_config(&dir.path().join("state.json")), compositor).unwrap();
        engine
            .pending_until
            .insert("terminal".into(), Instant::now() + Duration::from_secs(1));
        let response = engine
            .handle(ControlRequest::Show {
                scratchpad: "terminal".into(),
            })
            .unwrap();
        assert!(response.message.contains("already pending"));
        assert!(matches!(
            engine.compositor.actions.as_slice(),
            [Action::FocusWorkspace { .. }]
        ));
    }

    #[test]
    fn hide_restores_exact_origin_then_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut normal = workspace(1, "1", false);
        normal.active_window_id = Some(7);
        let compositor = FakeCompositor {
            workspaces: vec![normal, workspace(20, "scratch:terminal", true)],
            windows: vec![window(7, "editor", 1)],
            actions: vec![],
        };
        let mut engine =
            Engine::new(test_config(&dir.path().join("state.json")), compositor).unwrap();
        engine.state.origins.insert(
            "terminal".into(),
            ReturnTarget {
                workspace_id: 1,
                workspace_name: Some("1".into()),
                output_name: Some("eDP-1".into()),
                focused_window_id: Some(7),
            },
        );
        let response = engine
            .handle(ControlRequest::Toggle {
                scratchpad: "terminal".into(),
            })
            .unwrap();
        assert!(response.ok);
        assert!(matches!(
            engine.compositor.actions[0],
            Action::FocusWorkspace {
                reference: WorkspaceReferenceArg::Id(1)
            }
        ));
        assert!(matches!(
            engine.compositor.actions[1],
            Action::FocusWindow { id: 7 }
        ));
    }

    #[test]
    fn show_adopts_window_from_wrong_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let compositor = FakeCompositor {
            workspaces: vec![
                workspace(1, "1", true),
                workspace(20, "scratch:terminal", false),
            ],
            windows: vec![window(9, "scratch-terminal", 1)],
            actions: vec![],
        };
        let mut engine =
            Engine::new(test_config(&dir.path().join("state.json")), compositor).unwrap();
        engine
            .handle(ControlRequest::Show {
                scratchpad: "terminal".into(),
            })
            .unwrap();
        assert!(matches!(
            engine.compositor.actions[0],
            Action::MoveWindowToWorkspace {
                window_id: Some(9),
                focus: false,
                ..
            }
        ));
    }

    #[test]
    fn non_adopting_pad_ignores_matching_window_on_normal_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let compositor = FakeCompositor {
            workspaces: vec![
                workspace(1, "1", true),
                workspace(20, "scratch:terminal", false),
            ],
            windows: vec![window(9, "scratch-terminal", 1)],
            actions: vec![],
        };
        let mut config = test_config(&dir.path().join("state.json"));
        config
            .scratchpads
            .get_mut("terminal")
            .unwrap()
            .adopt_existing = false;
        let mut engine = Engine::new(config, compositor).unwrap();
        engine
            .handle(ControlRequest::Show {
                scratchpad: "terminal".into(),
            })
            .unwrap();
        assert!(matches!(
            engine.compositor.actions.as_slice(),
            [Action::FocusWorkspace { .. }, Action::Spawn { .. }]
        ));
    }
}
