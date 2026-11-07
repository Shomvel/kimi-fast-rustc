use std::process::Command;

fn kfrc(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_kfrc"))
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run kfrc {args:?}: {error}"))
}

fn assert_help(subcommand: Option<&str>) {
    for flag in ["--help", "-h"] {
        let args: Vec<&str> = subcommand.into_iter().chain([flag]).collect();
        let output = kfrc(&args);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "kfrc {args:?} failed with {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
        assert!(
            stdout.contains("usage:"),
            "kfrc {args:?} did not print usage\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

#[test]
fn root_help() {
    assert_help(None);
}

#[test]
fn native_help() {
    assert_help(Some("native"));
}

#[test]
fn native_batch_help() {
    assert_help(Some("native-batch"));
}

#[test]
fn tune_help() {
    assert_help(Some("tune"));
}

#[test]
fn cache_help() {
    assert_help(Some("cache"));
}

#[test]
fn unknown_subcommand_still_fails() {
    let output = kfrc(&["definitely-not-a-kfrc-subcommand"]);
    assert!(
        !output.status.success(),
        "unknown subcommand unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
