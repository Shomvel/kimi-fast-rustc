use std::env;
use std::error::Error as StdError;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::audit::AuditRecord;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Emit {
    pub kind: String,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RustcOptions {
    pub crate_types: Vec<String>,
    pub emit: Vec<Emit>,
    pub out_dir: Option<PathBuf>,
}

impl RustcOptions {
    pub fn parse(args: &[OsString]) -> Self {
        let mut options = Self::default();
        let mut index = 0;

        while index < args.len() {
            let arg = &args[index];
            match arg.to_str() {
                Some("--") => break,
                Some("--crate-type") => {
                    index += 1;
                    if let Some(value) = args.get(index).and_then(|value| value.to_str()) {
                        add_crate_types(&mut options.crate_types, value);
                    }
                }
                Some("--emit") => {
                    index += 1;
                    if let Some(value) = args.get(index).and_then(|value| value.to_str()) {
                        add_emit(&mut options.emit, value);
                    }
                }
                Some("--out-dir") => {
                    index += 1;
                    if let Some(value) = args.get(index) {
                        options.out_dir = Some(PathBuf::from(value));
                    }
                }
                Some(value) => {
                    if let Some(value) = value.strip_prefix("--crate-type=") {
                        add_crate_types(&mut options.crate_types, value);
                    } else if let Some(value) = value.strip_prefix("--emit=") {
                        add_emit(&mut options.emit, value);
                    } else if let Some(value) = value.strip_prefix("--out-dir=") {
                        options.out_dir = Some(PathBuf::from(value));
                    }
                }
                None => {}
            }
            index += 1;
        }

        options
    }

    pub fn is_proc_macro(&self) -> bool {
        self.crate_types
            .iter()
            .any(|crate_type| crate_type == "proc-macro")
    }
}

fn add_crate_types(crate_types: &mut Vec<String>, value: &str) {
    crate_types.extend(value.split(',').map(str::to_owned));
}

fn add_emit(emits: &mut Vec<Emit>, value: &str) {
    emits.extend(value.split(',').map(|item| {
        let (kind, path) = item.split_once('=').map_or((item, None), |(kind, path)| {
            (kind, Some(PathBuf::from(path)))
        });
        Emit {
            kind: kind.to_owned(),
            path,
        }
    }));
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RustcInvocation {
    pub rustc: PathBuf,
    pub args: Vec<OsString>,
    pub options: RustcOptions,
}

impl RustcInvocation {
    pub fn new<P, I, S>(rustc: P, args: I) -> Self
    where
        P: Into<PathBuf>,
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
        let options = RustcOptions::parse(&args);
        Self {
            rustc: rustc.into(),
            args,
            options,
        }
    }

    pub fn from_wrapper_args<I, S>(args: I) -> Result<Self, RustcError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let mut args = args.into_iter().map(Into::into);
        let rustc = args.next().ok_or(RustcError::MissingRustc)?;
        Ok(Self::new(PathBuf::from(rustc), args))
    }

    pub fn from_direct_args<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self::new(rustc_from_env(), args)
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.rustc);
        command
            .args(&self.args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        command
    }

    pub fn run(&self) -> Result<ExitStatus, RustcError> {
        self.command().status().map_err(|source| RustcError::Spawn {
            rustc: self.rustc.clone(),
            source,
        })
    }

    pub fn run_with_audit(
        &self,
        audit_path: Option<&Path>,
        strict: bool,
        backend: &str,
        profile: &str,
    ) -> Result<ExitStatus, RustcError> {
        if let Some(path) = audit_path {
            if let Err(source) = self.append_audit(path, backend, profile) {
                if strict {
                    return Err(RustcError::Audit {
                        path: path.to_path_buf(),
                        source,
                    });
                }
                eprintln!(
                    "kfrc: warning: failed to write audit record to {}: {source}",
                    path.display()
                );
            }
        }
        self.run()
    }

    fn append_audit(&self, path: &Path, backend: &str, profile: &str) -> io::Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let record = AuditRecord::new(&self.rustc, &self.args, backend, profile);
        let mut line = record.to_json_line();
        line.push('\n');
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        loop {
            match file.write(line.as_bytes()) {
                Ok(written) if written == line.len() => return Ok(()),
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "partial audit append; refusing a second write that could interleave records",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

pub fn select_rustc(rustc: Option<OsString>) -> PathBuf {
    rustc
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rustc"))
}

pub fn rustc_from_env() -> PathBuf {
    select_rustc(env::var_os("RUSTC"))
}

pub fn run() -> Result<ExitStatus, RustcError> {
    run_with_args(env::args_os().skip(1))
}

pub fn run_with_args<I, S>(args: I) -> Result<ExitStatus, RustcError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    RustcInvocation::from_wrapper_args(args)?.run()
}

pub fn exit_code(status: &ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }

    1
}

#[derive(Debug)]
pub enum RustcError {
    MissingRustc,
    Spawn { rustc: PathBuf, source: io::Error },
    Audit { path: PathBuf, source: io::Error },
}

impl fmt::Display for RustcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRustc => formatter.write_str(
                "invalid RUSTC_WRAPPER invocation: the first argument must be the rustc path",
            ),
            Self::Spawn { rustc, source } => {
                write!(formatter, "failed to execute {}: {source}", rustc.display())
            }
            Self::Audit { path, source } => write!(
                formatter,
                "audit failed for {} with KFRC_AUDIT_STRICT=1: {source}",
                path.display()
            ),
        }
    }
}

impl StdError for RustcError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::MissingRustc => None,
            Self::Spawn { source, .. } | Self::Audit { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn wrapper_uses_first_argument_as_rustc_and_preserves_every_other_argument() {
        let args = strings(&[
            "/toolchain/bin/rustc",
            "--crate-type",
            "lib,cdylib",
            "--emit=dep-info=/tmp/example.d,link",
            "--out-dir",
            "target/debug/deps",
            "-C",
            "opt-level=3",
            "src/lib.rs",
        ]);

        let invocation = RustcInvocation::from_wrapper_args(args.clone()).unwrap();

        assert_eq!(invocation.rustc, PathBuf::from("/toolchain/bin/rustc"));
        assert_eq!(invocation.args, args[1..]);
        assert_eq!(invocation.options.crate_types, ["lib", "cdylib"]);
        assert_eq!(
            invocation.options.emit,
            [
                Emit {
                    kind: "dep-info".into(),
                    path: Some(PathBuf::from("/tmp/example.d")),
                },
                Emit {
                    kind: "link".into(),
                    path: None,
                },
            ]
        );
        assert_eq!(
            invocation.options.out_dir,
            Some(PathBuf::from("target/debug/deps"))
        );
    }

    #[test]
    fn wrapper_rejects_an_invocation_without_rustc() {
        let error = RustcInvocation::from_wrapper_args(Vec::<OsString>::new()).unwrap_err();
        assert!(matches!(error, RustcError::MissingRustc));
    }

    #[test]
    fn parses_repeated_and_equals_forms_without_rewriting_arguments() {
        let args = strings(&[
            "--crate-type=rlib",
            "--crate-type",
            "dylib,staticlib",
            "--emit",
            "metadata,llvm-ir=module.ll",
            "--emit=obj=module.o,asm",
            "--out-dir=first",
            "--out-dir",
            "second",
        ]);

        let invocation = RustcInvocation::new("rustc", args.clone());

        assert_eq!(invocation.args, args);
        assert_eq!(
            invocation.options.crate_types,
            ["rlib", "dylib", "staticlib"]
        );
        assert_eq!(
            invocation.options.emit,
            [
                Emit {
                    kind: "metadata".into(),
                    path: None,
                },
                Emit {
                    kind: "llvm-ir".into(),
                    path: Some(PathBuf::from("module.ll")),
                },
                Emit {
                    kind: "obj".into(),
                    path: Some(PathBuf::from("module.o")),
                },
                Emit {
                    kind: "asm".into(),
                    path: None,
                },
            ]
        );
        assert_eq!(invocation.options.out_dir, Some(PathBuf::from("second")));
    }

    #[test]
    fn proc_macro_detection_matches_rustc_crate_type_syntax() {
        assert!(!RustcOptions::parse(&[]).is_proc_macro());
        for crate_type in [
            "bin",
            "lib",
            "rlib",
            "dylib",
            "cdylib",
            "staticlib",
            "proc_macro",
            "Proc-Macro",
            "PROC-MACRO",
        ] {
            let options = RustcOptions::parse(&strings(&["--crate-type", crate_type]));
            assert!(!options.is_proc_macro(), "crate type {crate_type}");
        }

        for args in [
            strings(&["--crate-type", "proc-macro"]),
            strings(&["--crate-type=proc-macro"]),
            strings(&["--crate-type=rlib,proc-macro"]),
            strings(&["--crate-type=rlib", "--crate-type", "proc-macro"]),
        ] {
            assert!(RustcOptions::parse(&args).is_proc_macro(), "{args:?}");
        }
    }

    #[test]
    fn option_parsing_stops_at_the_option_terminator() {
        let options = RustcOptions::parse(&strings(&[
            "--emit=link",
            "--",
            "--crate-type=cdylib",
            "--out-dir=ignored",
        ]));

        assert!(options.crate_types.is_empty());
        assert_eq!(options.emit[0].kind, "link");
        assert_eq!(options.out_dir, None);
    }

    #[test]
    fn direct_rustc_selection_honors_nonempty_rustc_and_has_a_default() {
        assert_eq!(select_rustc(None), PathBuf::from("rustc"));
        assert_eq!(select_rustc(Some(OsString::new())), PathBuf::from("rustc"));
        assert_eq!(
            select_rustc(Some(OsString::from("/custom/rustc"))),
            PathBuf::from("/custom/rustc")
        );
    }

    #[test]
    fn non_utf8_separate_out_dir_is_preserved() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;

            let path = OsString::from_vec(vec![b'o', b'u', b't', 0xff]);
            let options = RustcOptions::parse(&[OsString::from("--out-dir"), path.clone()]);
            assert_eq!(options.out_dir, Some(PathBuf::from(path)));
        }
    }

    #[cfg(unix)]
    #[test]
    fn executes_the_selected_rustc_and_returns_its_exit_status() {
        let args = strings(&["-c", "exit 37"]);
        let invocation = RustcInvocation::new("/bin/sh", args.clone());
        let status = invocation.run().unwrap();

        assert_eq!(status.code(), Some(37));
        assert_eq!(exit_code(&status), 37);
        assert_eq!(invocation.args, args);
    }

    #[cfg(unix)]
    #[test]
    fn signal_termination_maps_to_a_shell_compatible_exit_code() {
        let status = Command::new("sh")
            .args(["-c", "kill -TERM $$"])
            .status()
            .unwrap();
        assert_eq!(status.code(), None);
        assert_eq!(exit_code(&status), 128 + 15);
    }

    #[test]
    fn missing_executable_is_reported_as_a_spawn_error() {
        let invocation = RustcInvocation::new(
            env::temp_dir().join("kfrc-definitely-missing-rustc"),
            Vec::<OsString>::new(),
        );
        let error = invocation.run().unwrap_err();
        assert!(matches!(error, RustcError::Spawn { .. }));
        assert!(StdError::source(&error).is_some());
    }
}
