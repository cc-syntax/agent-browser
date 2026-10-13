//! Independently authored ABP-1 refusal acceptance; implementer must not edit.
//! Ticket/approved contract require debug observers to be refused before launch
//! or private-value dispatch. This calls the real CLI, not a mock privacy guard.
//! Runtime execution requires separately authorized isolated QA ownership.

#![cfg(unix)]

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_agent-browser");
const DIRECTORIES: &[&str] = &[
    "home",
    "sockets",
    "profile",
    "runtime",
    "tmp",
    "xdg-config",
    "xdg-data",
    "xdg-cache",
];

fn contains_value(bytes: &[u8], value: &[u8]) -> bool {
    bytes.windows(value.len()).any(|window| window == value)
}

// Only inspect the owned tree. Never follow a symlink or read a socket/device.
// Generic assertions deliberately never print paths, payloads or child output.
fn snapshot(root: &Path, value: &[u8]) -> Vec<(PathBuf, u32, Vec<u8>)> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).expect("cannot inspect owned acceptance directory") {
        let path = entry.expect("cannot inspect owned acceptance entry").path();
        let metadata = fs::symlink_metadata(&path).expect("cannot inspect owned entry metadata");
        let mode = metadata.permissions().mode() & 0o777;
        if metadata.is_dir() {
            assert!(mode == 0o700, "owned directory permissions changed");
            entries.push((path.clone(), mode, Vec::new()));
            entries.extend(snapshot(&path, value));
        } else {
            assert!(
                metadata.is_file(),
                "unexpected socket, symlink or device artifact"
            );
            assert!(mode == 0o600, "owned file permissions changed");
            let bytes = fs::read(&path).expect("cannot inspect owned acceptance file");
            assert!(!contains_value(&bytes, value), "private value persisted");
            entries.push((path, mode, bytes));
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn assert_private_debug_refusal(reason: &str) {
    // Accept safe equivalent wording rather than prescribing an implementation
    // API/type name. The known reason must identify BOTH private-session mode
    // and the incompatible debug observer, not just a parser/transport failure.
    // Example: "--private-input-session is incompatible with --debug".
    let reason = reason.to_ascii_lowercase();
    assert!(
        reason.contains("private")
            && reason.contains("session")
            && reason.contains("debug")
            && [
                "incompatible",
                "not supported",
                "unsupported",
                "cannot be used",
                "not allowed",
                "refuses",
                "forbidden",
            ]
            .iter()
            .any(|classification| reason.contains(classification))
            && !["unknown", "unrecognized", "missing", "usage:"]
                .iter()
                .any(|generic_error| reason.contains(generic_error)),
        "expected private-session debug incompatibility refusal, not generic parse failure"
    );
}

#[test]
fn private_stdin_fill_refuses_debug_before_launch_without_output_or_persistence() {
    // Fail before spawning on ANY inherited browser-tool configuration, even
    // an empty daemon/provider/CDP/socket variable. Do not sanitize authority or
    // silently connect to an ambient backend. HOME/CADENCE_* stay unchanged in
    // the test runner; only the future tool child gets an owned filesystem home.
    assert!(
        !std::env::vars_os().any(|(key, _)| {
            let key = key.to_string_lossy().to_ascii_uppercase();
            key.starts_with("AGENT_BROWSER_")
                || matches!(key.as_str(), "OBSCURA_BIN" | "CHROME_BIN" | "CHROME_PATH")
        }),
        "unexpected inherited browser-tool context; QA must provide an isolated launch"
    );

    // One bad-case acceptance, exercised in both canonical output modes.
    for json in [false, true] {
        let owned = tempfile::Builder::new()
            .prefix("abp1-refusal-")
            .tempdir_in("/tmp")
            .expect("cannot create owned acceptance root");
        fs::set_permissions(owned.path(), fs::Permissions::from_mode(0o700))
            .expect("cannot restrict owned acceptance root");
        for directory in DIRECTORIES {
            DirBuilder::new()
                .mode(0o700)
                .create(owned.path().join(directory))
                .expect("cannot create private acceptance directory");
        }
        let config = owned.path().join("config.json");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&config)
            .expect("cannot create owned configuration")
            .write_all(b"{}\n")
            .expect("cannot write owned configuration");

        // Synthetic unique value exists only in memory and the child's stdin.
        // Never put it in argv/environment, assertion text or an on-disk fixture.
        let value = format!("abp1-synthetic-{}", uuid::Uuid::new_v4());
        let before = snapshot(owned.path(), value.as_bytes());
        let mut command = Command::new(BIN);
        command
            .current_dir(owned.path())
            .env("HOME", owned.path().join("home"))
            .env("USERPROFILE", owned.path().join("home"))
            .env("XDG_CONFIG_HOME", owned.path().join("xdg-config"))
            .env("XDG_DATA_HOME", owned.path().join("xdg-data"))
            .env("XDG_CACHE_HOME", owned.path().join("xdg-cache"))
            .env("XDG_RUNTIME_DIR", owned.path().join("runtime"))
            .env("TMPDIR", owned.path().join("tmp"))
            .env("TMP", owned.path().join("tmp"))
            .env("TEMP", owned.path().join("tmp"))
            .env("AGENT_BROWSER_SOCKET_DIR", owned.path().join("sockets"))
            .env("NO_COLOR", "1")
            .arg("--config")
            .arg(&config)
            .args([
                "--session",
                "abp1-refusal",
                "--private-input-session",
                "--debug",
            ]);
        if json {
            command.arg("--json");
        }
        command
            .args(["fill", "#abp1-synthetic-password", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("cannot invoke owned source CLI");
        let mut stdin = child.stdin.take().expect("private stdin pipe unavailable");
        let write_result = stdin.write_all(value.as_bytes());
        drop(stdin);
        // An early refusal may close stdin before this small write. BrokenPipe
        // is acceptable only alongside the independently checked real refusal.
        assert!(
            write_result.is_ok()
                || write_result
                    .as_ref()
                    .is_err_and(|error| error.kind() == ErrorKind::BrokenPipe),
            "cannot provide synthetic private stdin"
        );
        let output = child
            .wait_with_output()
            .expect("cannot collect CLI refusal");
        assert!(
            !contains_value(&output.stdout, value.as_bytes())
                && !contains_value(&output.stderr, value.as_bytes()),
            "private value appeared in CLI output"
        );
        assert!(
            output.status.code() == Some(1),
            "expected normal refusal exit"
        );
        assert!(
            snapshot(owned.path(), value.as_bytes()) == before,
            "refusal created or changed a daemon, profile, stream, log, history or state artifact"
        );
        if json {
            let response: serde_json::Value = serde_json::from_slice(&output.stdout)
                .unwrap_or_else(|_| panic!("expected a structured JSON refusal"));
            assert!(
                response["success"] == false,
                "expected unsuccessful JSON response"
            );
            let reason = response["error"]
                .as_str()
                .expect("expected safe JSON refusal reason");
            assert_private_debug_refusal(reason);
        } else {
            assert!(
                output.stdout.is_empty(),
                "unexpected non-JSON refusal stdout"
            );
            let reason =
                std::str::from_utf8(&output.stderr).expect("expected safe UTF-8 refusal stderr");
            assert_private_debug_refusal(reason);
        }
    }
}
