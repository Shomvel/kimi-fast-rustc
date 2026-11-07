use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use kimi_fast_rustc::llvm::{LlvmConfig, OptLevel, TargetCpu};
use kimi_fast_rustc::rustc::{self, RustcInvocation};

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(message) => {
            eprintln!("kfrc: {message}");
            std::process::exit(2);
        }
    }
}

fn run() -> Result<i32, String> {
    let mut args = env::args_os().skip(1);
    let Some(first) = args.next() else {
        println!("kfrc {}", kimi_fast_rustc::VERSION);
        return Ok(0);
    };

    if first == "--version" || first == "-V" {
        println!("kfrc {}", kimi_fast_rustc::VERSION);
        return Ok(0);
    }
    if first == "--help" || first == "-h" {
        println!(
            "usage: kfrc <rustc-path> [rustc arguments]\n       kfrc rustc [rustc arguments]\n       kfrc native <input.rs> -o <output> [--emit=asm | --emit=obj | -S]\n       kfrc native <input.rs> [--emit=asm=<path> | --emit=obj=<path>]\n       kfrc native-batch --jobs N --out-dir DIR <input.kfr>...\n       kfrc fingerprint"
        );
        return Ok(0);
    }
    if first == "native" {
        kimi_fast_rustc::native_cli::run_with_args(args).map_err(|error| error.to_string())?;
        return Ok(0);
    }
    if first == "native-batch" {
        for output in run_native_batch(args)? {
            println!("{}", output.display());
        }
        return Ok(0);
    }
    if first == "fingerprint" {
        println!(
            "{}",
            kimi_fast_rustc::system::collect_system_info().to_json()
        );
        return Ok(0);
    }

    let direct = first == "rustc";
    let wrapper = !direct && is_rustc_path(Path::new(&first));
    if !direct && !wrapper {
        return Err(format!(
            "unsupported invocation {:?}; use --help for usage",
            first
        ));
    }
    if env::var("KFRC_BACKEND").is_ok_and(|value| value.eq_ignore_ascii_case("native")) {
        return Err("the native backend cannot compile a full Cargo crate yet".to_owned());
    }

    let invocation = if direct {
        RustcInvocation::from_direct_args(args)
    } else {
        RustcInvocation::from_wrapper_args(
            std::iter::once(first).chain(args.collect::<Vec<OsString>>()),
        )
        .map_err(|error| error.to_string())?
    };
    run_llvm_invocation(invocation, &WrapperOptions::from_env())
}

#[derive(Debug)]
struct NativeBatchOptions {
    jobs: usize,
    output_directory: PathBuf,
    inputs: Vec<PathBuf>,
}

fn run_native_batch<I, S>(args: I) -> Result<Vec<PathBuf>, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let options = parse_native_batch_args(args)?;
    kimi_fast_rustc::native_batch::compile_batch(
        &options.inputs,
        &options.output_directory,
        options.jobs,
    )
    .map_err(|error| error.to_string())
}

fn parse_native_batch_args<I, S>(args: I) -> Result<NativeBatchOptions, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let mut jobs = None;
    let mut output_directory = None;
    let mut inputs = Vec::new();

    while let Some(argument) = args.next() {
        if argument == "--jobs" {
            if jobs.is_some() {
                return Err("--jobs was provided more than once".to_owned());
            }
            let value = args
                .next()
                .ok_or_else(|| "missing value after --jobs".to_owned())?;
            let value = value
                .to_str()
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| {
                    format!("invalid --jobs value {value:?}; expected a positive integer")
                })?;
            if value == 0 {
                return Err("--jobs must be greater than zero".to_owned());
            }
            jobs = Some(value);
        } else if argument == "--out-dir" {
            if output_directory.is_some() {
                return Err("--out-dir was provided more than once".to_owned());
            }
            let path = args
                .next()
                .ok_or_else(|| "missing path after --out-dir".to_owned())?;
            if path.is_empty() {
                return Err("output directory after --out-dir must not be empty".to_owned());
            }
            output_directory = Some(PathBuf::from(path));
        } else if argument.to_string_lossy().starts_with('-') {
            return Err(format!("unknown native-batch option {argument:?}"));
        } else {
            if argument.is_empty() {
                return Err("input path must not be empty".to_owned());
            }
            inputs.push(PathBuf::from(argument));
        }
    }

    let jobs = jobs.ok_or_else(|| "missing --jobs N".to_owned())?;
    let output_directory = output_directory.ok_or_else(|| "missing --out-dir DIR".to_owned())?;
    if inputs.is_empty() {
        return Err("native-batch requires at least one input".to_owned());
    }
    Ok(NativeBatchOptions {
        jobs,
        output_directory,
        inputs,
    })
}

struct WrapperOptions {
    profile: String,
    target_cpu_native: bool,
    audit_path: Option<PathBuf>,
    audit_strict: bool,
}

impl WrapperOptions {
    fn from_env() -> Self {
        Self {
            profile: env::var("KFRC_PROFILE").unwrap_or_else(|_| "controlled".to_owned()),
            target_cpu_native: env::var("KFRC_TARGET_CPU").is_ok_and(|value| value == "native"),
            audit_path: env::var_os("KFRC_AUDIT")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            audit_strict: env::var_os("KFRC_AUDIT_STRICT").is_some_and(|value| value == "1"),
        }
    }
}

fn run_llvm_invocation(
    mut invocation: RustcInvocation,
    options: &WrapperOptions,
) -> Result<i32, String> {
    invocation.args.extend(
        profile_args(
            &options.profile,
            options.target_cpu_native,
            invocation.options.is_proc_macro(),
        )?
        .into_iter()
        .map(OsString::from),
    );
    let status = invocation
        .run_with_audit(
            options.audit_path.as_deref(),
            options.audit_strict,
            "llvm",
            &options.profile,
        )
        .map_err(|error| error.to_string())?;
    Ok(rustc::exit_code(&status))
}

fn is_rustc_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "rustc" || name.starts_with("rustc-"))
}

fn profile_args(
    profile: &str,
    target_cpu_native: bool,
    proc_macro: bool,
) -> Result<Vec<String>, String> {
    let mut config = match profile {
        "controlled" => return Ok(Vec::new()),
        "balanced" => LlvmConfig::default(),
        "speed" => LlvmConfig {
            opt_level: OptLevel::Speed,
            ..LlvmConfig::default()
        },
        "size" => LlvmConfig {
            opt_level: OptLevel::Size,
            ..LlvmConfig::default()
        },
        _ => return Err(format!("unknown KFRC_PROFILE {profile:?}")),
    };
    if target_cpu_native {
        config.target_cpu = TargetCpu::Native;
    }
    let args = config.rustc_args().map_err(|error| error.to_string())?;
    if !proc_macro {
        return Ok(args);
    }
    Ok(args
        .chunks(2)
        .filter(|pair| pair.get(1).is_none_or(|value| !value.starts_with("lto=")))
        .flatten()
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(unix)]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use kimi_fast_rustc::audit::AuditRecord;

    #[cfg(unix)]
    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    #[cfg(unix)]
    struct TestDirectory {
        path: PathBuf,
    }

    #[cfg(unix)]
    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "kfrc-wrapper-audit-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }
    }

    #[cfg(unix)]
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(unix)]
    struct FakeRustc {
        directory: TestDirectory,
        rustc: PathBuf,
        captured_args: PathBuf,
    }

    #[cfg(unix)]
    impl FakeRustc {
        fn new(exit_code: i32) -> Self {
            let directory = TestDirectory::new();
            let rustc = directory.path.join("rustc");
            let captured_args = directory.path.join("captured-args");
            fs::write(
                &rustc,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\nexit {exit_code}\n",
                    captured_args.display()
                ),
            )
            .unwrap();
            let mut permissions = fs::metadata(&rustc).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&rustc, permissions).unwrap();
            Self {
                directory,
                rustc,
                captured_args,
            }
        }

        fn args(&self) -> Vec<OsString> {
            let bytes = fs::read(&self.captured_args).unwrap();
            assert_eq!(bytes.last(), Some(&b'\n'));
            bytes[..bytes.len() - 1]
                .split(|byte| *byte == b'\n')
                .map(|arg| OsString::from_vec(arg.to_vec()))
                .collect()
        }

        fn run(
            &self,
            args: Vec<OsString>,
            profile: &str,
            audit_path: Option<PathBuf>,
            audit_strict: bool,
        ) -> Result<i32, String> {
            run_llvm_invocation(
                RustcInvocation::new(&self.rustc, args),
                &WrapperOptions {
                    profile: profile.to_owned(),
                    target_cpu_native: profile == "speed",
                    audit_path,
                    audit_strict,
                },
            )
        }
    }

    #[cfg(unix)]
    fn strings(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[cfg(unix)]
    fn assert_audit(path: &Path, rustc: &Path, args: &[OsString], profile: &str) {
        let content = fs::read_to_string(path).unwrap();
        assert!(content.ends_with('\n'));
        assert_eq!(content.lines().count(), 1);
        let expected = AuditRecord::new(rustc, args, "llvm", profile).to_json_line();
        assert_eq!(
            content.split(",\"pid\":").next(),
            expected.split(",\"pid\":").next()
        );
    }

    #[test]
    fn parses_native_batch_options_and_preserves_input_order() {
        let options = parse_native_batch_args([
            "first.kfr",
            "--out-dir",
            "objects",
            "second.kfr",
            "--jobs",
            "3",
        ])
        .unwrap();

        assert_eq!(options.jobs, 3);
        assert_eq!(options.output_directory, PathBuf::from("objects"));
        assert_eq!(
            options.inputs,
            [PathBuf::from("first.kfr"), PathBuf::from("second.kfr")]
        );
    }

    #[test]
    fn rejects_invalid_native_batch_arguments() {
        let cases = [
            (
                vec!["--jobs", "0", "--out-dir", "objects", "input.kfr"],
                "greater than zero",
            ),
            (
                vec!["--jobs", "many", "--out-dir", "objects", "input.kfr"],
                "positive integer",
            ),
            (
                vec!["--jobs", "2", "--out-dir", "objects"],
                "at least one input",
            ),
            (vec!["--out-dir", "objects", "input.kfr"], "missing --jobs"),
            (vec!["--jobs", "2", "input.kfr"], "missing --out-dir"),
            (
                vec!["--jobs", "2", "--out-dir", "", "input.kfr"],
                "must not be empty",
            ),
            (
                vec![
                    "--jobs",
                    "2",
                    "--jobs",
                    "3",
                    "--out-dir",
                    "objects",
                    "input.kfr",
                ],
                "more than once",
            ),
            (
                vec!["--jobs", "2", "--out-dir", "objects", "--unknown"],
                "unknown native-batch option",
            ),
        ];

        for (args, expected) in cases {
            let error = parse_native_batch_args(args).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn recognizes_only_rustc_executable_names() {
        assert!(is_rustc_path(Path::new("/toolchain/bin/rustc")));
        assert!(is_rustc_path(Path::new("rustc-1.94.1")));
        assert!(!is_rustc_path(Path::new("input.rs")));
        assert!(!is_rustc_path(Path::new("cargo")));
    }

    #[test]
    fn proc_macro_profiles_omit_only_lto() {
        let expected = [
            "-C",
            "opt-level=3",
            "-C",
            "codegen-units=1",
            "-C",
            "panic=abort",
            "-C",
            "strip=symbols",
            "-C",
            "target-cpu=native",
            "-C",
            "embed-bitcode=yes",
        ]
        .map(str::to_owned);

        assert_eq!(profile_args("speed", true, true).unwrap(), expected);
        for profile in ["balanced", "speed", "size"] {
            let args = profile_args(profile, false, true).unwrap();
            assert!(args.iter().all(|arg| !arg.starts_with("lto=")));
            assert!(args.iter().any(|arg| arg == "embed-bitcode=yes"));
            assert!(args.iter().any(|arg| arg.starts_with("opt-level=")));
            assert!(args.iter().any(|arg| arg.starts_with("codegen-units=")));
        }
    }

    #[cfg(unix)]
    #[test]
    fn controlled_wrapper_audit_matches_the_unchanged_forwarded_arguments() {
        let rustc = FakeRustc::new(0);
        let args = strings(&[
            "--crate-name",
            "controlled_demo",
            "--emit=dep-info=example.d,link",
            "-C",
            "opt-level=2",
            "src/lib.rs",
        ]);
        let audit = rustc.directory.path.join("nested/wrapper.jsonl");

        assert_eq!(
            rustc.run(args.clone(), "controlled", Some(audit.clone()), false),
            Ok(0)
        );

        assert_eq!(rustc.args(), args);
        assert_audit(&audit, &rustc.rustc, &args, "controlled");
    }

    #[cfg(unix)]
    #[test]
    fn controlled_proc_macro_wrapper_preserves_the_original_arguments() {
        let rustc = FakeRustc::new(0);
        let args = strings(&["--crate-type=proc-macro", "-C", "lto=thin", "input.rs"]);

        assert_eq!(rustc.run(args.clone(), "controlled", None, false), Ok(0));
        assert_eq!(rustc.args(), args);
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_filters_lto_only_for_proc_macro_crate_types() {
        let rustc = FakeRustc::new(0);
        let regular_profile = profile_args("speed", true, false)
            .unwrap()
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        let proc_macro_profile = profile_args("speed", true, true)
            .unwrap()
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();

        for crate_type in ["bin", "lib", "rlib", "dylib", "cdylib", "staticlib"] {
            let original = strings(&["--crate-type", crate_type, "input.rs"]);
            let mut final_args = original.clone();
            final_args.extend(regular_profile.iter().cloned());

            assert_eq!(rustc.run(original, "speed", None, false), Ok(0));
            assert_eq!(rustc.args(), final_args, "crate type {crate_type}");
        }

        for original in [
            strings(&["--crate-type", "proc-macro", "input.rs"]),
            strings(&["--crate-type=proc-macro", "input.rs"]),
            strings(&["--crate-type=rlib,proc-macro", "input.rs"]),
        ] {
            let mut final_args = original.clone();
            final_args.extend(proc_macro_profile.iter().cloned());

            assert_eq!(rustc.run(original, "speed", None, false), Ok(0));
            assert_eq!(rustc.args(), final_args);
        }
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_audit_contains_the_final_profile_arguments_and_invocation_metadata() {
        let rustc = FakeRustc::new(0);
        let original = strings(&["--crate-name=profiled", "--emit=link", "input.rs"]);
        let mut final_args = original.clone();
        final_args.extend(
            profile_args("speed", true, false)
                .unwrap()
                .into_iter()
                .map(OsString::from),
        );
        let audit = rustc.directory.path.join("profile.jsonl");

        assert_eq!(
            rustc.run(original, "speed", Some(audit.clone()), false),
            Ok(0)
        );

        assert_eq!(rustc.args(), final_args);
        assert_audit(&audit, &rustc.rustc, &final_args, "speed");
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_audit_preserves_non_utf8_arguments() {
        let rustc = FakeRustc::new(0);
        let args = vec![
            OsString::from("--crate-name=non_utf8"),
            OsString::from_vec(vec![b'a', b'r', b'g', 0x80, 0xff]),
        ];
        let audit = rustc.directory.path.join("non-utf8.jsonl");

        assert_eq!(
            rustc.run(args.clone(), "controlled", Some(audit.clone()), false),
            Ok(0)
        );

        assert_eq!(rustc.args(), args);
        assert_audit(&audit, &rustc.rustc, &args, "controlled");
        let json = fs::read_to_string(audit).unwrap();
        assert!(json.contains("{\"encoding\":\"hex\",\"value\":\"61726780ff\"}"));
    }

    #[cfg(unix)]
    #[test]
    fn audit_failure_warns_but_preserves_rustc_exit_status_and_arguments() {
        let rustc = FakeRustc::new(37);
        let args = strings(&["--crate-name=warning", "input.rs"]);
        let audit = rustc.directory.path.join("audit-is-a-directory");
        fs::create_dir(&audit).unwrap();

        assert_eq!(
            rustc.run(args.clone(), "controlled", Some(audit), false),
            Ok(37)
        );
        assert_eq!(rustc.args(), args);
    }

    #[cfg(unix)]
    #[test]
    fn strict_audit_failure_is_explicit_and_does_not_start_rustc() {
        let rustc = FakeRustc::new(0);
        let audit = rustc.directory.path.join("audit-is-a-directory");
        fs::create_dir(&audit).unwrap();

        let error = rustc
            .run(
                strings(&["--crate-name=strict", "input.rs"]),
                "controlled",
                Some(audit),
                true,
            )
            .unwrap_err();

        assert!(error.contains("audit failed"));
        assert!(error.contains("KFRC_AUDIT_STRICT=1"));
        assert!(!rustc.captured_args.exists());
    }
}
