//! Launch the sole Rust semantic backend through its pinned rustc frontend.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

pub const USAGE: &str = "usage: kfrc native <input.rs> [rustc arguments]\nSet KFRC_CODEGEN_BACKEND to librustc_codegen_kfrc.so and optionally KFRC_RUSTC to rustc 1.94.1.";

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<i32, String> {
    let args: Vec<_> = args.into_iter().collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return Ok(0);
    }
    if args.is_empty() {
        return Err(USAGE.into());
    }
    let backend = std::env::var_os("KFRC_CODEGEN_BACKEND")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("crates/rustc_codegen_kfrc/target/debug/librustc_codegen_kfrc.so")
        });
    let backend = backend.canonicalize().map_err(|error| format!(
        "semantic backend {}: {error}; build crates/rustc_codegen_kfrc with rustc 1.94.1 + rustc-dev, or set KFRC_CODEGEN_BACKEND", backend.display()
    ))?;
    let mut command = if let Some(rustc) = std::env::var_os("KFRC_RUSTC") {
        Command::new(rustc)
    } else {
        let mut command = Command::new("rustup");
        command.args(["run", "1.94.1", "rustc"]);
        command
    };
    let mut backend_flag = OsString::from("-Zcodegen-backend=");
    backend_flag.push(backend);
    let status = command
        .env("RUSTC_BOOTSTRAP", "1")
        .args(args)
        .args(["-Cpanic=abort", "-Zmir-opt-level=0"])
        .arg(backend_flag)
        .status()
        .map_err(|error| format!("starting pinned rustc semantic backend: {error}"))?;
    Ok(status.code().unwrap_or(1))
}
