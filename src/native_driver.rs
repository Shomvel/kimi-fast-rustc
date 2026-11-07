//! GNU x86_64 assembly output and native linking through the system toolchain.

use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const TEMP_DIRECTORY_ATTEMPTS: usize = 128;
static NEXT_TEMP_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeOutputKind {
    Assembly,
    Executable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeOutputOptions {
    pub output: PathBuf,
    pub kind: NativeOutputKind,
}

impl NativeOutputOptions {
    pub fn assembly(output: impl Into<PathBuf>) -> Self {
        Self {
            output: output.into(),
            kind: NativeOutputKind::Assembly,
        }
    }

    pub fn executable(output: impl Into<PathBuf>) -> Self {
        Self {
            output: output.into(),
            kind: NativeOutputKind::Executable,
        }
    }
}

impl Default for NativeOutputOptions {
    fn default() -> Self {
        Self::executable("a.out")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeStage {
    Assemble,
    Link,
}

impl NativeStage {
    fn description(self) -> &'static str {
        match self {
            Self::Assemble => "assembler",
            Self::Link => "linker",
        }
    }
}

#[derive(Debug)]
pub enum NativeDriverError {
    UnsupportedTarget {
        os: &'static str,
        arch: &'static str,
    },
    EmptyOutputPath,
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Spawn {
        stage: NativeStage,
        program: PathBuf,
        source: io::Error,
    },
    ToolFailed {
        stage: NativeStage,
        program: PathBuf,
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
}

impl NativeDriverError {
    pub fn status(&self) -> Option<ExitStatus> {
        match self {
            Self::ToolFailed { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn stdout(&self) -> Option<&[u8]> {
        match self {
            Self::ToolFailed { stdout, .. } => Some(stdout),
            _ => None,
        }
    }

    pub fn stderr(&self) -> Option<&[u8]> {
        match self {
            Self::ToolFailed { stderr, .. } => Some(stderr),
            _ => None,
        }
    }
}

impl fmt::Display for NativeDriverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTarget { os, arch } => write!(
                formatter,
                "native assembly is supported only on Linux x86_64, not {os} {arch}"
            ),
            Self::EmptyOutputPath => formatter.write_str("native output path must not be empty"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
            Self::Spawn {
                stage,
                program,
                source,
            } => write!(
                formatter,
                "failed to start {} {}: {source}",
                stage.description(),
                program.display()
            ),
            Self::ToolFailed {
                stage,
                status,
                stderr,
                ..
            } => {
                write!(formatter, "{} failed with {status}", stage.description())?;
                let stderr = String::from_utf8_lossy(stderr);
                let stderr = stderr.trim();
                if stderr.is_empty() {
                    Ok(())
                } else {
                    write!(formatter, ": {stderr}")
                }
            }
        }
    }
}

impl Error for NativeDriverError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::Spawn { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NativeDriver {
    assembler: PathBuf,
    linker: PathBuf,
    temp_root: PathBuf,
}

impl NativeDriver {
    pub fn new() -> Self {
        Self {
            assembler: PathBuf::from("as"),
            linker: PathBuf::from("gcc"),
            temp_root: env::temp_dir(),
        }
    }

    pub fn with_tools(mut self, assembler: impl Into<PathBuf>, linker: impl Into<PathBuf>) -> Self {
        self.assembler = assembler.into();
        self.linker = linker.into();
        self
    }

    pub fn with_temp_root(mut self, temp_root: impl Into<PathBuf>) -> Self {
        self.temp_root = temp_root.into();
        self
    }

    pub fn write_assembly(
        &self,
        assembly: &str,
        output: impl AsRef<Path>,
    ) -> Result<(), NativeDriverError> {
        let output = output.as_ref();
        validate_output_path(output)?;
        write_file(output, assembly.as_bytes())
    }

    pub fn compile(
        &self,
        assembly: &str,
        options: &NativeOutputOptions,
    ) -> Result<PathBuf, NativeDriverError> {
        validate_output_path(&options.output)?;
        if options.kind == NativeOutputKind::Assembly {
            self.write_assembly(assembly, &options.output)?;
            return Ok(options.output.clone());
        }
        ensure_native_target()?;

        let temporary = TempDirectory::create(&self.temp_root)?;
        let source = temporary.path.join("input.s");
        let object = temporary.path.join("input.o");
        write_file(&source, assembly.as_bytes())?;

        self.run_tool(NativeStage::Assemble, &self.assembler, |command| {
            command.arg("--64").arg("-o").arg(&object).arg(&source);
        })?;
        self.run_tool(NativeStage::Link, &self.linker, |command| {
            command.arg("-o").arg(&options.output).arg(&object);
        })?;

        Ok(options.output.clone())
    }

    fn run_tool(
        &self,
        stage: NativeStage,
        program: &Path,
        arguments: impl FnOnce(&mut Command),
    ) -> Result<(), NativeDriverError> {
        let mut command = Command::new(program);
        arguments(&mut command);
        let output = command
            .output()
            .map_err(|source| NativeDriverError::Spawn {
                stage,
                program: program.to_path_buf(),
                source,
            })?;
        if output.status.success() {
            return Ok(());
        }

        Err(NativeDriverError::ToolFailed {
            stage,
            program: program.to_path_buf(),
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

impl Default for NativeDriver {
    fn default() -> Self {
        Self::new()
    }
}

fn ensure_native_target() -> Result<(), NativeDriverError> {
    if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        Ok(())
    } else {
        Err(NativeDriverError::UnsupportedTarget {
            os: env::consts::OS,
            arch: env::consts::ARCH,
        })
    }
}

fn validate_output_path(output: &Path) -> Result<(), NativeDriverError> {
    if output.as_os_str().is_empty() {
        Err(NativeDriverError::EmptyOutputPath)
    } else {
        Ok(())
    }
}

fn write_file(path: &Path, contents: &[u8]) -> Result<(), NativeDriverError> {
    fs::write(path, contents).map_err(|source| NativeDriverError::Io {
        operation: "write file",
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug)]
struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn create(root: &Path) -> Result<Self, NativeDriverError> {
        for _ in 0..TEMP_DIRECTORY_ATTEMPTS {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos());
            let id = NEXT_TEMP_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "kfrc-native-{}-{timestamp:x}-{id:x}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(NativeDriverError::Io {
                        operation: "create temporary directory",
                        path,
                        source,
                    });
                }
            }
        }

        Err(NativeDriverError::Io {
            operation: "create a unique temporary directory in",
            path: root.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "too many temporary directory name collisions",
            ),
        })
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXIT_42: &str = r#".text
.globl main
.type main, @function
main:
    movl $42, %eax
    ret
.section .note.GNU-stack,"",@progbits
"#;

    fn assert_temp_root_is_empty(root: &Path) {
        let entries = fs::read_dir(root)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let leftovers = entries
            .iter()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("kfrc-native-"))
            })
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "temporary files were not cleaned up: {leftovers:?}"
        );
    }

    #[test]
    fn output_options_select_the_artifact_kind() {
        let assembly = NativeOutputOptions::assembly("program.s");
        assert_eq!(assembly.kind, NativeOutputKind::Assembly);
        assert_eq!(assembly.output, Path::new("program.s"));

        let executable = NativeOutputOptions::executable("program");
        assert_eq!(executable.kind, NativeOutputKind::Executable);
        assert_eq!(NativeOutputOptions::default().output, Path::new("a.out"));
    }

    #[test]
    fn assembly_only_writes_the_source_without_running_tools() {
        let root = TempDirectory::create(&env::temp_dir()).unwrap();
        let output = root.path.join("program.s");
        let driver = NativeDriver::new().with_tools("missing-as", "missing-gcc");

        let returned = driver
            .compile(EXIT_42, &NativeOutputOptions::assembly(&output))
            .unwrap();

        assert_eq!(returned, output);
        assert_eq!(fs::read_to_string(output).unwrap(), EXIT_42);
        assert_temp_root_is_empty(&root.path);
    }

    #[test]
    fn temporary_directories_are_unique_and_removed_on_drop() {
        let root = TempDirectory::create(&env::temp_dir()).unwrap();
        let first = TempDirectory::create(&root.path).unwrap();
        let second = TempDirectory::create(&root.path).unwrap();
        let first_path = first.path.clone();
        let second_path = second.path.clone();

        assert_ne!(first_path, second_path);
        assert!(first_path.is_dir());
        assert!(second_path.is_dir());
        drop(first);
        drop(second);
        assert!(!first_path.exists());
        assert!(!second_path.exists());
    }

    #[test]
    fn empty_output_path_is_rejected() {
        let error = NativeDriver::new()
            .compile("", &NativeOutputOptions::assembly(""))
            .unwrap_err();
        assert!(matches!(error, NativeDriverError::EmptyOutputPath));
    }

    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    #[test]
    fn executable_compilation_rejects_unsupported_targets() {
        let error = NativeDriver::new()
            .compile(EXIT_42, &NativeOutputOptions::executable("program"))
            .unwrap_err();
        assert!(matches!(error, NativeDriverError::UnsupportedTarget { .. }));
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn tool_failure_preserves_output_status_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempDirectory::create(&env::temp_dir()).unwrap();
        let assembler = root.path.join("failing-as");
        fs::write(
            &assembler,
            "#!/bin/sh\necho assembler-stdout\necho assembler-stderr >&2\nexit 23\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&assembler).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&assembler, permissions).unwrap();
        let output = root.path.join("program");
        let driver = NativeDriver::new()
            .with_tools(&assembler, "gcc")
            .with_temp_root(&root.path);

        let error = driver
            .compile(EXIT_42, &NativeOutputOptions::executable(&output))
            .unwrap_err();

        match error {
            NativeDriverError::ToolFailed {
                stage,
                status,
                stdout,
                stderr,
                ..
            } => {
                assert_eq!(stage, NativeStage::Assemble);
                assert_eq!(status.code(), Some(23));
                assert_eq!(stdout, b"assembler-stdout\n");
                assert_eq!(stderr, b"assembler-stderr\n");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!output.exists());
        assert_temp_root_is_empty(&root.path);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn missing_tool_reports_spawn_failure_and_cleans_up() {
        let root = TempDirectory::create(&env::temp_dir()).unwrap();
        let missing = root.path.join("missing-as");
        let driver = NativeDriver::new()
            .with_tools(&missing, "gcc")
            .with_temp_root(&root.path);

        let error = driver
            .compile(
                EXIT_42,
                &NativeOutputOptions::executable(root.path.join("program")),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            NativeDriverError::Spawn {
                stage: NativeStage::Assemble,
                ..
            }
        ));
        assert_temp_root_is_empty(&root.path);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn builds_and_runs_an_executable_and_preserves_it() {
        let root = TempDirectory::create(&env::temp_dir()).unwrap();
        let output = root.path.join("program");
        let driver = NativeDriver::new().with_temp_root(&root.path);

        let returned = driver
            .compile(EXIT_42, &NativeOutputOptions::executable(&output))
            .unwrap();

        assert_eq!(returned, output);
        assert!(output.is_file());
        let status = Command::new(&output).status().unwrap();
        assert_eq!(status.code(), Some(42));
        assert_temp_root_is_empty(&root.path);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn invalid_assembly_reports_diagnostics_and_cleans_up() {
        let root = TempDirectory::create(&env::temp_dir()).unwrap();
        let output = root.path.join("program");
        let driver = NativeDriver::new().with_temp_root(&root.path);

        let error = driver
            .compile(
                ".text\nthis_is_not_an_instruction\n",
                &NativeOutputOptions::executable(&output),
            )
            .unwrap_err();

        assert_eq!(error.status().and_then(|status| status.code()), Some(1));
        assert!(!error.stderr().unwrap().is_empty());
        assert!(!output.exists());
        assert_temp_root_is_empty(&root.path);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn linker_failure_reports_the_link_stage_and_cleans_up() {
        let root = TempDirectory::create(&env::temp_dir()).unwrap();
        let output = root.path.join("program");
        let driver = NativeDriver::new().with_temp_root(&root.path);
        let assembly = ".text\n.globl helper\nhelper:\n    ret\n";

        let error = driver
            .compile(assembly, &NativeOutputOptions::executable(&output))
            .unwrap_err();

        match error {
            NativeDriverError::ToolFailed { stage, stderr, .. } => {
                assert_eq!(stage, NativeStage::Link);
                assert!(!stderr.is_empty());
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!output.exists());
        assert_temp_root_is_empty(&root.path);
    }
}
