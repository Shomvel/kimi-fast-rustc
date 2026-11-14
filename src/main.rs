use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use kimi_fast_rustc::cache_admin::{CacheAdmin, OutputFormat};
use kimi_fast_rustc::llvm::{LlvmConfig, Lto, OptLevel, TargetCpu};
use kimi_fast_rustc::rustc::{self, RustcInvocation};
use kimi_fast_rustc::tuning_runner::{self, TuningRunner};

const CACHE_USAGE: &str =
    "usage: kfrc cache --root DIR stats [--json]\n       kfrc cache --root DIR clear [--json]";
const TUNE_USAGE: &str = "usage: kfrc tune <crate-dir|Cargo.toml> --out-dir DIR [--candidates N] [--runs N] [--workload PROGRAM [ARG...]]";

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
            "usage: kfrc <rustc-path> [rustc arguments]\n       kfrc rustc [rustc arguments]\n       kfrc native <input.rs> [rustc arguments]\n       kfrc tune <crate-dir|Cargo.toml> --out-dir DIR [--candidates N] [--runs N] [--workload PROGRAM [ARG...]]\n       kfrc cache --root DIR stats [--json]\n       kfrc cache --root DIR clear [--json]\n       kfrc fingerprint"
        );
        return Ok(0);
    }
    if first == "tune" {
        return run_tune(args);
    }
    if first == "cache" {
        return run_cache(args);
    }
    if first == "native" {
        return kimi_fast_rustc::semantic_cli::run(args);
    }
    if first == "native-batch" {
        return Err("native-batch used the removed demonstration IR; compile real Rust with `kfrc native`, or use Cargo's package runner".into());
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheOperation {
    Stats,
    Clear,
}

#[derive(Debug)]
struct CacheOptions {
    root: PathBuf,
    operation: CacheOperation,
    format: OutputFormat,
}

fn run_cache<I, S>(args: I) -> Result<i32, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{CACHE_USAGE}");
        return Ok(0);
    }
    let options = parse_cache_args(args)?;
    println!("{}", execute_cache(&options)?);
    Ok(0)
}

fn parse_cache_args<I, S>(args: I) -> Result<CacheOptions, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let mut root = None;
    let mut operation = None;
    let mut json = false;

    while let Some(argument) = args.next() {
        if argument == "--root" {
            if root.is_some() {
                return Err("--root was provided more than once".to_owned());
            }
            let path = args
                .next()
                .ok_or_else(|| "missing path after --root".to_owned())?;
            if path.is_empty() {
                return Err("cache root after --root must not be empty".to_owned());
            }
            root = Some(PathBuf::from(path));
        } else if argument == "--json" {
            if json {
                return Err("--json was provided more than once".to_owned());
            }
            json = true;
        } else if argument == "stats" || argument == "clear" {
            if operation.is_some() {
                return Err("cache accepts exactly one operation: stats or clear".to_owned());
            }
            operation = Some(if argument == "stats" {
                CacheOperation::Stats
            } else {
                CacheOperation::Clear
            });
        } else if argument.to_string_lossy().starts_with('-') {
            return Err(format!("unknown cache option {argument:?}"));
        } else {
            return Err(format!(
                "unknown cache operation {argument:?}; expected stats or clear"
            ));
        }
    }

    Ok(CacheOptions {
        root: root.ok_or_else(|| "missing --root DIR".to_owned())?,
        operation: operation
            .ok_or_else(|| "cache requires an operation: stats or clear".to_owned())?,
        format: if json {
            OutputFormat::Json
        } else {
            OutputFormat::Human
        },
    })
}

fn execute_cache(options: &CacheOptions) -> Result<String, String> {
    let cache = CacheAdmin::new(&options.root);
    let (operation, result) = match options.operation {
        CacheOperation::Stats => (
            "stats",
            cache.stats().map(|stats| stats.format(options.format)),
        ),
        CacheOperation::Clear => (
            "clear",
            cache.clear().map(|report| report.format(options.format)),
        ),
    };
    result.map_err(|error| {
        format!(
            "cache {operation} failed for {}: {error}",
            options.root.display()
        )
    })
}

#[derive(Debug)]
struct TuneOptions {
    input: PathBuf,
    candidates: usize,
    runs: usize,
    output_directory: PathBuf,
    workload_program: Option<PathBuf>,
    workload_args: Vec<String>,
}

fn run_tune<I, S>(args: I) -> Result<i32, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{TUNE_USAGE}");
        println!(
            "defaults: --candidates {} --runs {}; --workload must be the final tune option",
            tuning_runner::DEFAULT_MAX_CANDIDATES,
            tuning_runner::MIN_RUNTIME_SAMPLES
        );
        println!(
            "the workload defaults to the built binary; {{artifact}} in workload arguments expands to each binary"
        );
        return Ok(0);
    }
    let options = parse_tune_args(args)?;
    execute_tune(&options)
}

fn parse_tune_args<I, S>(args: I) -> Result<TuneOptions, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let mut input = None;
    let mut candidates = None;
    let mut runs = None;
    let mut output_directory = None;
    let mut workload_program = None;
    let mut workload_args = Vec::new();

    while let Some(argument) = args.next() {
        if argument == "--candidates" {
            if candidates.is_some() {
                return Err("--candidates was provided more than once".to_owned());
            }
            let value = args
                .next()
                .ok_or_else(|| "missing value after --candidates".to_owned())?;
            let value = value
                .to_str()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    format!("invalid --candidates value {value:?}; expected a positive integer")
                })?;
            candidates = Some(value);
        } else if argument == "--runs" {
            if runs.is_some() {
                return Err("--runs was provided more than once".to_owned());
            }
            let value = args
                .next()
                .ok_or_else(|| "missing value after --runs".to_owned())?;
            let value = value
                .to_str()
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| format!("invalid --runs value {value:?}; expected an integer"))?;
            if value < tuning_runner::MIN_RUNTIME_SAMPLES {
                return Err(format!(
                    "--runs must be at least {}",
                    tuning_runner::MIN_RUNTIME_SAMPLES
                ));
            }
            runs = Some(value);
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
        } else if argument == "--workload" {
            let mut workload = args.collect::<Vec<_>>();
            if workload.is_empty() {
                return Err(
                    "--workload requires a program and may be followed by arguments".to_owned(),
                );
            }
            let program = workload.remove(0);
            if program.is_empty() {
                return Err("workload program must not be empty".to_owned());
            }
            if program.to_str().is_none() {
                return Err("workload program path must be valid UTF-8".to_owned());
            }
            workload_args = workload
                .into_iter()
                .map(|argument| {
                    argument.into_string().map_err(|argument| {
                        format!("workload argument must be valid UTF-8: {argument:?}")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            workload_program = Some(PathBuf::from(program));
            break;
        } else if argument.to_string_lossy().starts_with('-') {
            return Err(format!("unknown tune option {argument:?}"));
        } else {
            if argument.is_empty() {
                return Err("crate input path must not be empty".to_owned());
            }
            if input.is_some() {
                return Err("tune accepts exactly one crate input".to_owned());
            }
            input = Some(PathBuf::from(argument));
        }
    }

    Ok(TuneOptions {
        input: input.ok_or_else(|| "tune requires a crate directory or Cargo.toml".to_owned())?,
        candidates: candidates.unwrap_or(tuning_runner::DEFAULT_MAX_CANDIDATES),
        runs: runs.unwrap_or(tuning_runner::MIN_RUNTIME_SAMPLES),
        output_directory: output_directory.ok_or_else(|| "missing --out-dir DIR".to_owned())?,
        workload_program,
        workload_args,
    })
}

fn execute_tune(options: &TuneOptions) -> Result<i32, String> {
    let (manifest_path, artifact_relative_path) = resolve_tune_input(&options.input)?;
    let mut runner = TuningRunner::new(
        manifest_path,
        &options.output_directory,
        artifact_relative_path,
    );
    runner.max_candidates = options.candidates;
    runner.runs = options.runs;
    runner.workload_program = options.workload_program.clone();
    runner.benchmark_args = options.workload_args.clone();

    let report = runner.run_default().map_err(|error| error.to_string())?;
    let json = report.to_json();
    let report_path = report.run_dir.join("report.json");
    fs::write(&report_path, format!("{json}\n")).map_err(|error| {
        format!(
            "could not write tuning report {}: {error}",
            report_path.display()
        )
    })?;
    println!("{json}");

    if report.has_measurement_failures() {
        eprintln!(
            "kfrc: one or more tuning measurements failed; evidence: {}",
            report_path.display()
        );
        Ok(1)
    } else {
        Ok(0)
    }
}

fn resolve_tune_input(input: &Path) -> Result<(PathBuf, PathBuf), String> {
    let manifest_path = if input.is_dir() {
        input.join("Cargo.toml")
    } else {
        input.to_path_buf()
    };
    if manifest_path
        .file_name()
        .is_none_or(|name| name != "Cargo.toml")
    {
        return Err(format!(
            "tune input must be a crate directory or Cargo.toml: {}",
            input.display()
        ));
    }
    let manifest = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "could not read crate manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let binary_name = manifest_binary_name(&manifest).ok_or_else(|| {
        format!(
            "could not determine a binary name from [package] or [[bin]] in {}",
            manifest_path.display()
        )
    })?;
    if binary_name == "." || binary_name == ".." || binary_name.contains(['/', '\\']) {
        return Err("crate binary name must not contain path components".to_owned());
    }
    let artifact_name = format!("{binary_name}{}", env::consts::EXE_SUFFIX);
    Ok((manifest_path, PathBuf::from("release").join(artifact_name)))
}

fn manifest_binary_name(manifest: &str) -> Option<String> {
    let mut section = "";
    let mut package_name = None;
    let mut binary_name = None;

    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with("[[") && line.ends_with("]]") {
            section = line[2..line.len() - 2].trim();
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "name" {
            continue;
        }
        let value = value.trim();
        let name = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        match section {
            "package" if package_name.is_none() => package_name = Some(name.to_owned()),
            "bin" if binary_name.is_none() => binary_name = Some(name.to_owned()),
            _ => {}
        }
    }

    binary_name.or(package_name)
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
    if let Ok(value) = env::var("KFRC_CODEGEN_UNITS") {
        config.codegen_units = value
            .parse()
            .map_err(|_| format!("invalid KFRC_CODEGEN_UNITS {value:?}"))?;
    }
    if let Ok(value) = env::var("KFRC_LTO") {
        config.lto = match value.as_str() {
            "thin" => Lto::Thin,
            "fat" => Lto::Fat,
            _ => return Err(format!("invalid KFRC_LTO {value:?}; expected thin or fat")),
        };
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
    fn parses_cache_operations_and_output_formats() {
        let stats = parse_cache_args(["--root", "cache", "stats"]).unwrap();
        assert_eq!(stats.root, PathBuf::from("cache"));
        assert_eq!(stats.operation, CacheOperation::Stats);
        assert_eq!(stats.format, OutputFormat::Human);

        let clear = parse_cache_args(["--root", "cache", "clear", "--json"]).unwrap();
        assert_eq!(clear.operation, CacheOperation::Clear);
        assert_eq!(clear.format, OutputFormat::Json);

        assert!(CACHE_USAGE.contains("cache --root DIR stats [--json]"));
        assert!(CACHE_USAGE.contains("cache --root DIR clear [--json]"));
    }

    #[test]
    fn rejects_invalid_cache_arguments() {
        let cases = [
            (vec![], "missing --root"),
            (vec!["stats"], "missing --root"),
            (vec!["--root", "cache"], "requires an operation"),
            (vec!["--root"], "missing path"),
            (vec!["--root", "", "stats"], "must not be empty"),
            (
                vec!["--root", "first", "--root", "second", "stats"],
                "more than once",
            ),
            (
                vec!["--root", "cache", "stats", "clear"],
                "exactly one operation",
            ),
            (
                vec!["--root", "cache", "stats", "--json", "--json"],
                "more than once",
            ),
            (
                vec!["--root", "cache", "stats", "--unknown"],
                "unknown cache option",
            ),
            (
                vec!["--root", "cache", "unknown"],
                "unknown cache operation",
            ),
        ];

        for (args, expected) in cases {
            let error = parse_cache_args(args).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn cache_commands_print_reports_and_clear_preserves_the_root() {
        let temp = TestDirectory::new();
        let root = temp.path.join("cache");
        let prefix = root.join("aa");
        fs::create_dir_all(&prefix).unwrap();
        fs::write(prefix.join("corrupt.o"), b"corrupt").unwrap();

        let stats = CacheOptions {
            root: root.clone(),
            operation: CacheOperation::Stats,
            format: OutputFormat::Human,
        };
        let report = execute_cache(&stats).unwrap();
        assert!(report.contains("Entries: 1"));
        assert!(report.contains("Total size: 7 B (7 bytes)"));
        assert!(report.contains("Corrupt entries: 1"));

        let stats = CacheOptions {
            format: OutputFormat::Json,
            ..stats
        };
        assert_eq!(
            execute_cache(&stats).unwrap(),
            "{\"entries\":1,\"total_bytes\":7,\"corrupt_entries\":1}"
        );

        let clear = CacheOptions {
            root: root.clone(),
            operation: CacheOperation::Clear,
            format: OutputFormat::Json,
        };
        assert_eq!(
            execute_cache(&clear).unwrap(),
            "{\"removed_files\":1,\"removed_directories\":1,\"removed_bytes\":7}"
        );
        assert!(root.is_dir());
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn cache_commands_reject_dangerous_roots() {
        let clear_root = CacheOptions {
            root: PathBuf::from("/"),
            operation: CacheOperation::Clear,
            format: OutputFormat::Json,
        };
        let error = execute_cache(&clear_root).unwrap_err();
        assert!(
            error.contains("refusing to use a filesystem root"),
            "{error}"
        );

        let temp = TestDirectory::new();
        let file = temp.path.join("not-a-directory");
        fs::write(&file, b"keep").unwrap();
        let stats_file = CacheOptions {
            root: file.clone(),
            operation: CacheOperation::Stats,
            format: OutputFormat::Human,
        };
        let error = execute_cache(&stats_file).unwrap_err();
        assert!(error.contains("must be a directory"), "{error}");
        assert_eq!(fs::read(file).unwrap(), b"keep");
    }

    #[test]
    fn parses_tune_defaults_and_workload_command() {
        let defaults = parse_tune_args(["small-crate", "--out-dir", "tuned"]).unwrap();
        assert_eq!(defaults.input, PathBuf::from("small-crate"));
        assert_eq!(defaults.output_directory, PathBuf::from("tuned"));
        assert_eq!(defaults.candidates, tuning_runner::DEFAULT_MAX_CANDIDATES);
        assert_eq!(defaults.runs, tuning_runner::MIN_RUNTIME_SAMPLES);
        assert_eq!(defaults.workload_program, None);
        assert!(defaults.workload_args.is_empty());

        let options = parse_tune_args([
            "--runs",
            "4",
            "--candidates",
            "2",
            "--out-dir",
            "tuned",
            "small-crate",
            "--workload",
            "/bin/sh",
            "-c",
            "exit 0",
        ])
        .unwrap();
        assert_eq!(options.candidates, 2);
        assert_eq!(options.runs, 4);
        assert_eq!(options.workload_program, Some(PathBuf::from("/bin/sh")));
        assert_eq!(options.workload_args, ["-c", "exit 0"]);
    }

    #[test]
    fn rejects_missing_or_unsafe_tune_arguments() {
        let cases = [
            (vec![], "requires a crate"),
            (vec!["small-crate"], "missing --out-dir"),
            (vec!["--out-dir", "tuned"], "requires a crate"),
            (
                vec!["small-crate", "--out-dir", "tuned", "--runs", "2"],
                "at least 3",
            ),
            (
                vec!["small-crate", "--out-dir", "tuned", "--candidates", "0"],
                "positive integer",
            ),
            (
                vec!["small-crate", "--out-dir", "tuned", "--runs", "many"],
                "expected an integer",
            ),
            (
                vec!["small-crate", "--out-dir", "tuned", "--out-dir", "other"],
                "more than once",
            ),
            (
                vec!["small-crate", "--out-dir", "tuned", "--rustflags"],
                "unknown tune option",
            ),
            (
                vec!["first", "second", "--out-dir", "tuned"],
                "exactly one crate input",
            ),
            (
                vec!["small-crate", "--out-dir", "tuned", "--workload"],
                "requires a program",
            ),
            (vec!["small-crate", "--out-dir", ""], "must not be empty"),
        ];

        for (args, expected) in cases {
            let error = parse_tune_args(args).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn determines_small_crate_binary_names_from_manifests() {
        assert_eq!(
            manifest_binary_name(
                "[package]\nname = \"tiny-demo\" # comment\nversion = \"0.1.0\"\n"
            ),
            Some("tiny-demo".to_owned())
        );
        assert_eq!(
            manifest_binary_name(
                "[package]\nname = 'package-name'\n\n[[bin]]\nname = \"actual-demo\"\n"
            ),
            Some("actual-demo".to_owned())
        );
        assert_eq!(manifest_binary_name("[package]\nname = \"\"\n"), None);
        assert_eq!(manifest_binary_name("[package]\nname = 42\n"), None);
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
    fn wrapper_audit_directory_separates_crates_in_warn_and_strict_modes() {
        let rustc = FakeRustc::new(0);
        let audit = rustc.directory.path.join("audit-directory");
        fs::create_dir(&audit).unwrap();

        for (crate_name, strict) in [("warn_crate", false), ("strict_crate", true)] {
            let args = vec![
                OsString::from(format!("--crate-name={crate_name}")),
                OsString::from("input.rs"),
            ];
            assert_eq!(
                rustc.run(args.clone(), "controlled", Some(audit.clone()), strict),
                Ok(0)
            );
            assert_eq!(rustc.args(), args);
            assert_audit(
                &audit.join(format!("crate-{crate_name}.jsonl")),
                &rustc.rustc,
                &args,
                "controlled",
            );
        }

        assert_eq!(fs::read_dir(audit).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_audit_directory_preserves_non_utf8_crate_names_and_arguments() {
        let rustc = FakeRustc::new(0);
        let args = vec![
            OsString::from("--crate-name"),
            OsString::from_vec(vec![b'n', b'o', b'n', b'_', b'u', b't', b'f', b'8', 0xff]),
            OsString::from_vec(vec![b'a', b'r', b'g', 0x80, 0xff]),
        ];
        let audit = rustc.directory.path.join("non-utf8-directory");
        fs::create_dir(&audit).unwrap();

        assert_eq!(
            rustc.run(args.clone(), "controlled", Some(audit.clone()), true),
            Ok(0)
        );

        assert_eq!(rustc.args(), args);
        let audit = audit.join("crate-non_utf8_ff.jsonl");
        assert_audit(&audit, &rustc.rustc, &args, "controlled");
        let json = fs::read_to_string(audit).unwrap();
        assert!(json.contains("{\"encoding\":\"hex\",\"value\":\"6e6f6e5f75746638ff\"}"));
        assert!(json.contains("{\"encoding\":\"hex\",\"value\":\"61726780ff\"}"));
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
        let audit = rustc.directory.path.join("audit-is-a-directory.jsonl");
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
        let audit = rustc.directory.path.join("audit-is-a-directory.jsonl");
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
