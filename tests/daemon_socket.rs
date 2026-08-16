use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use niri_ipc::{Reply, Request, Response, Workspace};
use niri_scratch::{Config, ControlRequest, send_control};

#[test]
fn daemon_round_trip_toggles_against_fake_niri() {
    let dir = tempfile::tempdir().unwrap();
    let niri_path = dir.path().join("niri.sock");
    let control_path = dir.path().join("control.sock");
    let state_path = dir.path().join("state.json");
    let config_path = dir.path().join("config.toml");
    fs::write(
        &config_path,
        format!(
            r#"
[daemon]
socket = "{}"
state_file = "{}"
launch_timeout_ms = 0

[scratchpads.terminal]
workspace = "scratch:terminal"
command = ["kitty", "--class", "niri-scratch-terminal"]
match_app_id = "^niri-scratch-terminal$"
"#,
            control_path.display(),
            state_path.display()
        ),
    )
    .unwrap();

    let fake_listener = UnixListener::bind(&niri_path).unwrap();
    let fake = thread::spawn(move || {
        for stream in fake_listener.incoming().take(4) {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            let response: Reply = Ok(match request {
                Request::Workspaces => Response::Workspaces(vec![
                    Workspace {
                        id: 1,
                        idx: 1,
                        name: Some("1".into()),
                        output: Some("eDP-1".into()),
                        is_urgent: false,
                        is_active: true,
                        is_focused: true,
                        active_window_id: None,
                    },
                    Workspace {
                        id: 20,
                        idx: 7,
                        name: Some("scratch:terminal".into()),
                        output: Some("eDP-1".into()),
                        is_urgent: false,
                        is_active: false,
                        is_focused: false,
                        active_window_id: None,
                    },
                ]),
                Request::Windows => Response::Windows(vec![]),
                Request::Action(_) => Response::Handled,
                other => panic!("unexpected fake Niri request: {other:?}"),
            });
            serde_json::to_writer(&mut stream, &response).unwrap();
            stream.write_all(b"\n").unwrap();
        }
    });

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_niri-scratch"))
        .args(["--config", config_path.to_str().unwrap(), "daemon"])
        .env("NIRI_SOCKET", &niri_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    while !control_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        control_path.exists(),
        "daemon did not create control socket"
    );

    let config = Config::load(&config_path).unwrap();
    let response = send_control(
        &config,
        &ControlRequest::Toggle {
            scratchpad: "terminal".into(),
        },
    )
    .unwrap();
    assert!(response.ok, "{}", response.message);
    assert!(response.message.contains("application launched"));
    let state = fs::read_to_string(&state_path).unwrap();
    assert!(state.contains("terminal"));

    daemon.kill().unwrap();
    daemon.wait().unwrap();
    fake.join().unwrap();
}
