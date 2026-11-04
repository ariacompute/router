//! End-to-end CLI tests: run the built binary as a subprocess with an isolated
//! `ARIA_COMPUTE_HOME`, so `main()` (exit codes, stderr, clap wiring) is covered
//! without touching the real user profile.

use std::io::Write;
use std::process::{Command, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_aria-router"))
}

/// Run the CLI with a scripted stdin (no tty).
fn run_with_stdin(home: &std::path::Path, args: &[&str], stdin: &str) -> std::process::Output {
    use std::io::Read;

    let mut child = bin()
        .env("ARIA_COMPUTE_HOME", home)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aria-router");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    // Closing stdin before waiting avoids a deadlock on the prompt loop.
    drop(child.stdin.take());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child
        .stdout
        .as_mut()
        .expect("stdout")
        .read_to_end(&mut stdout)
        .expect("read stdout");
    child
        .stderr
        .as_mut()
        .expect("stderr")
        .read_to_end(&mut stderr)
        .expect("read stderr");
    let status = child.wait().expect("wait");
    std::process::Output {
        status,
        stdout,
        stderr,
    }
}

#[test]
fn version_flag_and_subcommand_print_the_version() {
    let out = bin().arg("--version").output().expect("run --version");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("aria-router"), "{stdout}");

    let out = bin().arg("version").output().expect("run version");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("aria-router "), "{stdout}");
}

#[test]
fn setup_validate_status_and_failure_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    // Non-interactive setup (stdin is not a TTY).
    let out = bin()
        .env("ARIA_COMPUTE_HOME", home)
        .args([
            "setup",
            "--template",
            "semantic",
            "--admin-user",
            "admin",
            "--admin-password",
            "password1",
        ])
        .output()
        .expect("run setup");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.join("router.yml").is_file());
    assert!(home.join("router-users.json").is_file());

    // validate (default path) → "ok"
    let out = bin()
        .env("ARIA_COMPUTE_HOME", home)
        .arg("validate")
        .output()
        .expect("run validate");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");

    // setup --status prints the resolved paths.
    let out = bin()
        .env("ARIA_COMPUTE_HOME", home)
        .args(["setup", "--status"])
        .output()
        .expect("run setup --status");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("config:"), "{stdout}");
    assert!(stdout.contains("require_api_key: true"), "{stdout}");
    assert!(stdout.contains("upgrade_url: https://"), "{stdout}");

    // Invalid YAML → exit code 1 with a message on stderr.
    let bad = home.join("bad.yml");
    std::fs::write(&bad, "version: v0.3\nbogus_key: 1\n").unwrap();
    let out = bin()
        .env("ARIA_COMPUTE_HOME", home)
        .args(["validate", "--config", bad.to_str().unwrap()])
        .output()
        .expect("run validate (bad)");
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(1));
    assert!(!out.stderr.is_empty(), "expected a diagnostic on stderr");

    // Missing default config → actionable error, still exit 1.
    let empty = tempfile::tempdir().unwrap();
    let out = bin()
        .env("ARIA_COMPUTE_HOME", empty.path())
        .arg("validate")
        .output()
        .expect("run validate (no config)");
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing --config"));

    // No arguments at all → clap usage error.
    let out = bin()
        .env("ARIA_COMPUTE_HOME", home)
        .output()
        .expect("run without args");
    assert!(!out.status.success());
}

#[test]
fn setup_prompts_through_stdio_without_a_tty() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    // `--clear` always asks for confirmation → `StdIo::prompt` reads stdin.
    let out = run_with_stdin(home, &["setup", "--clear"], "n\n");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // admin password + confirmation are read even without a tty.
    let out = run_with_stdin(
        home,
        &["setup", "--template", "semantic", "--admin-user", "admin"],
        "password1\npassword1\n",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.join("router.yml").is_file());
    assert!(home.join("router-users.json").is_file());
}

#[cfg(unix)]
#[test]
fn setup_reads_the_password_from_a_real_tty_with_masked_echo() {
    use std::os::fd::FromRawFd;

    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    let (master, slave) = unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master >= 0, "posix_openpt failed");
        let name = libc::ptsname(master);
        assert!(!name.is_null(), "ptsname failed");
        assert_eq!(libc::grantpt(master), 0);
        assert_eq!(libc::unlockpt(master), 0);
        let slave = libc::open(name, libc::O_RDWR | libc::O_NOCTTY);
        assert!(slave >= 0, "open pts failed");
        (
            std::fs::File::from_raw_fd(master),
            std::fs::File::from_raw_fd(slave),
        )
    };

    // Every other value comes from a flag, so only the two masked password
    // prompts run (`isatty(stdin) == true`).
    let mut child = bin()
        .env("ARIA_COMPUTE_HOME", home)
        .args([
            "setup",
            "--template",
            "semantic",
            "--admin-user",
            "admin",
            "--base-url",
            "http://127.0.0.1:9",
            "--api-key-env",
            "GATEWAY_API_KEY",
            "--api-key",
            "sk-pty",
            "--model-small",
            "s",
            "--model-mid",
            "m",
            "--model-large",
            "l",
            "--upgrade-url",
            "https://github.com/acme/router",
        ])
        .stdin(Stdio::from(slave))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn aria-router");

    let mut writer = master;
    // Both prompts use raw (no-echo, no-canonical) reads; the writes may race
    // with process exit, so failures are irrelevant.
    let _ = writer.write_all(b"password1\npassword1\n");
    let _ = writer.flush();

    let status = child.wait().expect("wait");
    assert!(status.success(), "masked prompt setup failed");
    assert!(home.join("router.yml").is_file());
    assert!(home.join("router-users.json").is_file());
}
