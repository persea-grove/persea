//! persea#340 regression tests: session-create resource cleanup and caps.
//!
//! - P1: a guacd handshake failure after a web session's browser spawn
//!   must kill the Xvnc/Chromium processes, free the display slot, and
//!   remove the Chromium profile dir — no early `?` may skip cleanup.
//!   The browser stand-ins are shell scripts (a fake Xvnc that listens on
//!   the VNC port, a fake Chromium that just stays alive), so the create
//!   path reaches the guacd handshake with real resources allocated.
//! - P2: the per-user cap is checked inside the sessions-map write lock,
//!   so two concurrent creates for one identity cannot both pass.
//! - P2: an RDP drive directory created on the create path is removed
//!   when creation later fails.

mod support;

use persea::protocol::{Instruction, InstructionParser};
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const ASSERT_DEADLINE: Duration = Duration::from_secs(15);
const TEST_KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

// ── Test environment (mirrors tests/hardening_tests.rs) ─────────────

/// Double-submit CSRF: GET the app root, capture the `csrf_token` cookie
/// value, and echo it back as `X-CSRF-Token` on state-changing requests.
async fn fetch_csrf_token(client: &reqwest::Client, base: &str) -> String {
    let resp = client
        .get(format!("{base}/"))
        .send()
        .await
        .expect("GET / for CSRF token");
    let set_cookie = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with("csrf_token="))
        .unwrap_or_else(|| panic!("no csrf_token Set-Cookie in {:?}", resp.headers()));
    set_cookie
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("csrf_token=")
        .unwrap()
        .to_string()
}

async fn send_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    key: &str,
    body: &serde_json::Value,
    csrf: Option<&str>,
) -> (reqwest::StatusCode, String) {
    let mut request = client.request(method, url).bearer_auth(key).json(body);
    if let Some(tok) = csrf {
        request = request.header("X-CSRF-Token", tok);
        request = request.header("Cookie", format!("csrf_token={tok}"));
    }
    let resp = request
        .send()
        .await
        .unwrap_or_else(|e| panic!("request to {url} failed: {e}"));
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

// ── Mock guacd (minimal handshake: args to select, ready to connect) ─

async fn mock_guacd(listener: tokio::net::TcpListener, args_names: Vec<String>) {
    loop {
        let (mut sock, _) = match listener.accept().await {
            Ok(x) => x,
            Err(_) => return,
        };
        let args_names = args_names.clone();
        tokio::spawn(async move {
            let mut parser = InstructionParser::new();
            let mut buf = [0u8; 4096];
            loop {
                match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        for parsed in parser.receive(&text) {
                            let Ok(instr) = parsed else { continue };
                            match instr.opcode.as_str() {
                                "select" => {
                                    let reply = Instruction::new("args", args_names.clone());
                                    let _ = sock.write_all(reply.encode().as_bytes()).await;
                                }
                                "connect" => {
                                    let ready = Instruction::new("ready", vec!["mock".into()]);
                                    let _ = sock.write_all(ready.encode().as_bytes()).await;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        });
    }
}

// ── Boot helper ─────────────────────────────────────────────────────

/// Which guacd endpoint the booted server talks to.
enum Guacd {
    /// A live mock that completes the guacamole handshake.
    Mock,
    /// A bound-then-released port: every connect fails.
    Dead,
}

struct TestEnv {
    base: String,
    key: String,
    csrf: String,
    client: reqwest::Client,
    _tmp: PathBuf,
    _app: support::AppProc,
}

/// Boot persea with the given extra config (verbatim TOML appended after
/// the top-level keys; put section headers last). Top-level keys must
/// come before any section header the caller includes.
async fn boot(tag: &str, extra_config: &str, guacd: Guacd) -> TestEnv {
    let marker = format!("s340-{tag}-{}", std::process::id());
    let tmp = std::env::temp_dir().join(&marker);
    std::fs::create_dir_all(&tmp).expect("create scratch dir");
    let config_path = tmp.join("config.toml");
    let log_path = tmp.join("persea.log");

    let guacd_addr = match guacd {
        Guacd::Mock => {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock guacd");
            let addr = listener.local_addr().expect("guacd addr");
            tokio::spawn(mock_guacd(
                listener,
                vec![
                    "hostname".into(),
                    "port".into(),
                    "username".into(),
                    "password".into(),
                    "width".into(),
                    "height".into(),
                    "dpi".into(),
                ],
            ));
            addr.to_string()
        }
        Guacd::Dead => {
            // Port 1 is never a guacd: binding a port and dropping it would
            // let a parallel test's ephemeral bind claim it and answer for
            // guacd, and the kernel never hands out port 1.
            "127.0.0.1:1".to_string()
        }
    };

    let db_path = tmp.join("admin.db").display().to_string();
    let booted = support::boot_persea(
        &marker,
        &config_path,
        &log_path,
        None,
        HEALTH_TIMEOUT,
        &|port: u16| {
            format!(
                "listen_addr = \"127.0.0.1:{port}\"\ndb_path = \"{db_path}\"\nguacd_addr = \"{guacd_addr}\"\n{extra_config}\n\n[storage]\nencryption_key = \"{TEST_KEY_HEX}\""
            )
        },
    )
    .await;
    let csrf = fetch_csrf_token(&booted.client, &booted.base).await;

    TestEnv {
        base: booted.base,
        key: booted.key,
        csrf,
        client: booted.client,
        _tmp: tmp,
        _app: booted.app,
    }
}

/// POST a session-create body; returns (status, body).
async fn create_session(env: &TestEnv, body: &serde_json::Value) -> (reqwest::StatusCode, String) {
    send_json(
        &env.client,
        reqwest::Method::POST,
        &format!("{}/api/sessions", env.base),
        &env.key,
        body,
        Some(&env.csrf),
    )
    .await
}

// ── P2: per-user cap under concurrency ──────────────────────────────

#[tokio::test]
async fn per_user_cap_rejects_second_of_two_concurrent_creates() {
    let env = boot("cap", "max_sessions_per_user = 1", Guacd::Mock).await;

    let body = json!({
        "session_type": "ssh",
        "hostname": "127.0.0.1",
        "port": 22,
        "username": "u",
        "password": "p",
    });

    // Two creates for the SAME API-key identity racing each other: the
    // cap check now runs under the insert lock, so exactly one may win.
    let (r1, r2) = tokio::join!(create_session(&env, &body), create_session(&env, &body));

    let wins = [&r1, &r2].iter().filter(|r| r.0.is_success()).count();
    assert_eq!(
        wins, 1,
        "exactly one concurrent create may succeed: statuses {} / {}, bodies: {} | {}",
        r1.0, r2.0, r1.1, r2.1
    );

    let loser = if r1.0.is_success() { &r2 } else { &r1 };
    assert_eq!(
        loser.0, 400,
        "the losing create must be rejected with 400: {}",
        loser.1
    );
    assert!(
        loser.1.contains("maximum sessions per user reached"),
        "expected the per-user cap message, got: {}",
        loser.1
    );
}

// ── P1: web session browser cleanup on guacd handshake failure ──────

/// All persea Chromium profile dirs currently on disk.
fn chromium_profile_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("persea-chromium-")
            {
                dirs.push(entry.path());
            }
        }
    }
    dirs
}

/// PIDs whose /proc/<pid>/cmdline contains `needle` (Linux-only, no
/// external pgrep dependency).
fn processes_matching(needle: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return pids;
    };
    for entry in entries.flatten() {
        let pid: u32 = match entry.file_name().to_str().and_then(|s| s.parse().ok()) {
            Some(p) => p,
            None => continue,
        };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if String::from_utf8_lossy(&cmdline).contains(needle) {
            pids.push(pid);
        }
    }
    pids
}

async fn wait_for<F: Fn() -> bool>(what: &str, cond: F) {
    let deadline = Instant::now() + ASSERT_DEADLINE;
    loop {
        if cond() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what} ({ASSERT_DEADLINE:?})"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn web_session_guacd_failure_kills_browser_and_frees_display() {
    // Precondition: the fake browser stack shells out to python3 for the
    // listening fake Xvnc. Fail loudly rather than weakening the test.
    assert!(
        std::process::Command::new("python3")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        "python3 is required for the fake Xvnc stand-in"
    );

    // Scripts live in their own marker dir so /proc scanning only matches
    // these processes (the persea server cmdline carries config.toml from
    // a different directory).
    let scripts_dir =
        std::env::temp_dir().join(format!("persea340-browser-{}", std::process::id()));
    std::fs::create_dir_all(&scripts_dir).expect("create scripts dir");
    let start_log = scripts_dir.join("starts.log");

    // Fake Xvnc: logs each start (proof the display slot was reused),
    // then listens on 5900+<display> so the spawn port-readiness check
    // passes. `exec` keeps it the direct child of persea so kill_on_drop
    // reaches it; the marker path stays on its cmdline as an argument.
    // The python one-liner `while` avoids block indentation, which the
    // Rust string continuations would strip.
    let xvnc_path = scripts_dir.join("xvnc.sh");
    let xvnc_src = format!(
        "#!/bin/sh\n\
         echo xvnc >> \"{log}\"\n\
         exec python3 - \"$1\" \"{marker}\" <<'PYEOF'\n\
         import socket, sys, time\n\
         port = 5900 + int(sys.argv[1].lstrip(':'))\n\
         s = socket.socket()\n\
         s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n\
         s.bind(('127.0.0.1', port))\n\
         s.listen(8)\n\
         while True: time.sleep(30)\n\
         PYEOF\n",
        log = start_log.display(),
        marker = scripts_dir.display()
    );
    std::fs::write(&xvnc_path, xvnc_src).expect("write xvnc stand-in");

    // Fake Chromium: logs each start, then stays alive (persea's 500ms
    // liveness check) until killed.
    let chromium_path = scripts_dir.join("chromium.sh");
    let chromium_src = format!(
        "#!/bin/sh\n\
         echo chromium >> \"{log}\"\n\
         while :; do sleep 1; done\n",
        log = start_log.display()
    );
    std::fs::write(&chromium_path, chromium_src).expect("write chromium stand-in");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [&xvnc_path, &chromium_path] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
                .expect("chmod browser stand-in");
        }
    }

    // Two display slots (validation requires start < end, and the range
    // is inclusive): two failed creates can exhaust both slots if they
    // leak, so the THIRD create only reaches browser spawn again if both
    // were freed.
    let extra = format!(
        "xvnc_path = \"{}\"\nchromium_path = \"{}\"\ndisplay_range_start = 0\ndisplay_range_end = 1",
        xvnc_path.display(),
        chromium_path.display()
    );
    let env = boot("webleak", &extra, Guacd::Dead).await;

    let profiles_before = chromium_profile_dirs();
    let web_body = json!({
        "session_type": "web",
        "url": "http://127.0.0.1/",
        "username": "u",
        "password": "p",
    });

    let xvnc_starts = || -> usize {
        std::fs::read_to_string(&start_log)
            .unwrap_or_default()
            .lines()
            .filter(|l| *l == "xvnc")
            .count()
    };

    // First create: browser stack spawns fully, then the guacd handshake
    // fails (dead port). Everything must be torn down.
    let (status, body) = create_session(&env, &web_body).await;
    assert_eq!(status, 502, "create must fail at guacd: {body}");

    wait_for("Xvnc/Chromium stand-ins killed", || {
        processes_matching(&scripts_dir.display().to_string()).is_empty()
    })
    .await;
    wait_for("no leaked Chromium profile dir", || {
        chromium_profile_dirs()
            .iter()
            .all(|d| profiles_before.contains(d))
    })
    .await;

    // Repeated failures must keep spawning (slots freed each time): with
    // two slots in the range, exhaustion shows up as the start log
    // stalling below 3. Then the last attempt's processes must die too.
    let (status, body) = create_session(&env, &web_body).await;
    assert_eq!(status, 502, "second create must fail at guacd: {body}");
    let (status, body) = create_session(&env, &web_body).await;
    assert_eq!(status, 502, "third create must fail at guacd: {body}");
    wait_for("display slots freed and browser respawned", || {
        xvnc_starts() >= 3
    })
    .await;
    wait_for("last attempt's processes killed", || {
        processes_matching(&scripts_dir.display().to_string()).is_empty()
    })
    .await;
    wait_for("no leaked Chromium profile dir after retries", || {
        chromium_profile_dirs()
            .iter()
            .all(|d| profiles_before.contains(d))
    })
    .await;
}

// ── P2: RDP drive directory cleanup on create failure ───────────────

#[tokio::test]
async fn rdp_drive_dir_removed_when_create_fails() {
    let tmp = std::env::temp_dir().join(format!("s340-drive-{}", std::process::id()));
    let drive_path = tmp.join("drives");
    std::fs::create_dir_all(&drive_path).expect("create drive base dir");

    let extra = format!(
        "[drive]\nenabled = true\ndrive_path = \"{}\"\ncleanup_on_close = true\n",
        drive_path.display()
    );
    let env = boot("driveleak", &extra, Guacd::Dead).await;

    // The drive dir is created in the RDP branch BEFORE the guacd
    // handshake; the handshake failure must take the directory with it.
    let (status, body) = create_session(
        &env,
        &json!({
            "session_type": "rdp",
            "hostname": "127.0.0.1",
            "port": 3389,
            "username": "u",
            "password": "p",
            "enable_drive": true,
        }),
    )
    .await;
    assert_eq!(status, 502, "create must fail at guacd: {body}");

    wait_for("failed create leaves no per-session drive dir", || {
        std::fs::read_dir(&drive_path)
            .map(|entries| {
                entries
                    .flatten()
                    .all(|e| uuid::Uuid::parse_str(&e.file_name().to_string_lossy()).is_err())
            })
            .unwrap_or(true)
    })
    .await;
}
