//! LLVM reference-track profile-guided optimization.
//!
//! The native backend has no LLVM tools or subprocesses and must not call this
//! module. A pipeline uses one work directory for instrumented profiles and one
//! known executable for training.

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

const RAW_PROFILE_DIRECTORY: &str = "profraw";
const RAW_PROFILE_PATTERN: &str = "default_%m_%p.profraw";
const MERGED_PROFILE: &str = "merged.profdata";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgoStage {
    Instrument,
    Train,
    Merge,
    ProfileUse,
}

impl PgoStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Instrument => "instrument",
            Self::Train => "train",
            Self::Merge => "llvm-profdata merge",
            Self::ProfileUse => "profile-use",
        }
    }
}

impl fmt::Display for PgoStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub enum PgoError {
    Io {
        stage: PgoStage,
        path: PathBuf,
        source: io::Error,
    },
    CommandFailed {
        stage: PgoStage,
        program: PathBuf,
        status: ExitStatus,
    },
    NoRawProfiles {
        directory: PathBuf,
    },
    MissingProfdata {
        path: PathBuf,
    },
    ProfdataNotAFile {
        path: PathBuf,
    },
    EmptyProfdata {
        path: PathBuf,
    },
}

impl fmt::Display for PgoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                stage,
                path,
                source,
            } => write!(
                formatter,
                "PGO {stage} I/O failed for {}: {source}",
                path.display()
            ),
            Self::CommandFailed {
                stage,
                program,
                status,
            } => write!(
                formatter,
                "PGO {stage} command {} exited with {status}",
                program.display()
            ),
            Self::NoRawProfiles { directory } => write!(
                formatter,
                "no .profraw files found in {}",
                directory.display()
            ),
            Self::MissingProfdata { path } => {
                write!(formatter, "PGO profile {} does not exist", path.display())
            }
            Self::ProfdataNotAFile { path } => {
                write!(formatter, "PGO profile {} is not a file", path.display())
            }
            Self::EmptyProfdata { path } => {
                write!(formatter, "PGO profile {} is empty", path.display())
            }
        }
    }
}

impl Error for PgoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
}

impl CommandSpec {
    fn new(program: &Path) -> Self {
        Self {
            program: program.to_path_buf(),
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    pub fn env(&self) -> &[(OsString, OsString)] {
        &self.env
    }

    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command.envs(self.env.iter().cloned());
        command
    }

    fn execute(&self, stage: PgoStage) -> Result<(), PgoError> {
        let status = self.to_command().status().map_err(|source| PgoError::Io {
            stage,
            path: self.program.clone(),
            source,
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(PgoError::CommandFailed {
                stage,
                program: self.program.clone(),
                status,
            })
        }
    }
}

#[derive(Debug, Clone)]
pub struct PgoPipeline {
    rustc: PathBuf,
    llvm_profdata: PathBuf,
    work_dir: PathBuf,
    instrumented_binary: PathBuf,
}

impl PgoPipeline {
    pub fn new(work_dir: impl Into<PathBuf>, instrumented_binary: impl Into<PathBuf>) -> Self {
        Self {
            rustc: PathBuf::from("rustc"),
            llvm_profdata: PathBuf::from("llvm-profdata"),
            work_dir: work_dir.into(),
            instrumented_binary: instrumented_binary.into(),
        }
    }

    pub fn with_tools(
        mut self,
        rustc: impl Into<PathBuf>,
        llvm_profdata: impl Into<PathBuf>,
    ) -> Self {
        self.rustc = rustc.into();
        self.llvm_profdata = llvm_profdata.into();
        self
    }

    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    pub fn raw_profile_dir(&self) -> PathBuf {
        self.work_dir.join(RAW_PROFILE_DIRECTORY)
    }

    pub fn raw_profile_pattern(&self) -> PathBuf {
        self.raw_profile_dir().join(RAW_PROFILE_PATTERN)
    }

    pub fn profdata_path(&self) -> PathBuf {
        self.work_dir.join(MERGED_PROFILE)
    }

    pub fn instrumented_binary(&self) -> &Path {
        &self.instrumented_binary
    }

    pub fn instrument_command<I, S>(&self, rustc_args: I) -> CommandSpec
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = CommandSpec::new(&self.rustc);
        command.args.extend(collect_args(rustc_args));
        command
            .args
            .push(path_flag("-Cprofile-generate=", &self.raw_profile_dir()));
        command.args.push(OsString::from("-o"));
        command
            .args
            .push(self.instrumented_binary.as_os_str().to_owned());
        command
    }

    pub fn training_command<I, S>(&self, training_args: I) -> CommandSpec
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = CommandSpec::new(&self.instrumented_binary);
        command.args.extend(collect_args(training_args));
        command.env.push((
            OsString::from("LLVM_PROFILE_FILE"),
            self.raw_profile_pattern().into_os_string(),
        ));
        command
    }

    pub fn discover_raw_profiles(&self) -> Result<Vec<PathBuf>, PgoError> {
        let directory = self.raw_profile_dir();
        let entries = fs::read_dir(&directory).map_err(|source| PgoError::Io {
            stage: PgoStage::Merge,
            path: directory.clone(),
            source,
        })?;
        let mut profiles = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| PgoError::Io {
                stage: PgoStage::Merge,
                path: directory.clone(),
                source,
            })?;
            let path = entry.path();
            if path.extension() == Some(OsStr::new("profraw")) && path.is_file() {
                profiles.push(path);
            }
        }
        profiles.sort();
        if profiles.is_empty() {
            Err(PgoError::NoRawProfiles { directory })
        } else {
            Ok(profiles)
        }
    }

    pub fn merge_command(&self, raw_profiles: &[PathBuf]) -> Result<CommandSpec, PgoError> {
        if raw_profiles.is_empty() {
            return Err(PgoError::NoRawProfiles {
                directory: self.raw_profile_dir(),
            });
        }
        let mut command = CommandSpec::new(&self.llvm_profdata);
        command.args.push(OsString::from("merge"));
        command
            .args
            .push(path_flag("-output=", &self.profdata_path()));
        command.args.extend(
            raw_profiles
                .iter()
                .map(|profile| profile.as_os_str().to_owned()),
        );
        Ok(command)
    }

    pub fn profile_use_command<I, S>(&self, rustc_args: I) -> Result<CommandSpec, PgoError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.validate_profdata()?;
        let mut command = CommandSpec::new(&self.rustc);
        command.args.extend(collect_args(rustc_args));
        command
            .args
            .push(path_flag("-Cprofile-use=", &self.profdata_path()));
        Ok(command)
    }

    pub fn validate_profdata(&self) -> Result<(), PgoError> {
        let path = self.profdata_path();
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(PgoError::MissingProfdata { path });
            }
            Err(source) => {
                return Err(PgoError::Io {
                    stage: PgoStage::ProfileUse,
                    path,
                    source,
                });
            }
        };
        if !metadata.is_file() {
            Err(PgoError::ProfdataNotAFile { path })
        } else if metadata.len() == 0 {
            Err(PgoError::EmptyProfdata { path })
        } else {
            Ok(())
        }
    }

    pub fn run_instrument<I, S>(&self, rustc_args: I) -> Result<(), PgoError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let directory = self.raw_profile_dir();
        fs::create_dir_all(&directory).map_err(|source| PgoError::Io {
            stage: PgoStage::Instrument,
            path: directory,
            source,
        })?;
        self.instrument_command(rustc_args)
            .execute(PgoStage::Instrument)
    }

    pub fn run_training<I, S>(&self, training_args: I) -> Result<(), PgoError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.training_command(training_args)
            .execute(PgoStage::Train)
    }

    pub fn run_merge(&self) -> Result<(), PgoError> {
        let profiles = self.discover_raw_profiles()?;
        self.merge_command(&profiles)?.execute(PgoStage::Merge)?;
        self.validate_profdata()
    }

    pub fn run_profile_use<I, S>(&self, rustc_args: I) -> Result<(), PgoError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.profile_use_command(rustc_args)?
            .execute(PgoStage::ProfileUse)
    }

    pub fn run<I, S, J, T, K, U>(
        &self,
        instrument_args: I,
        training_args: J,
        profile_use_args: K,
    ) -> Result<(), PgoError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
        J: IntoIterator<Item = T>,
        T: AsRef<OsStr>,
        K: IntoIterator<Item = U>,
        U: AsRef<OsStr>,
    {
        self.run_instrument(instrument_args)?;
        self.run_training(training_args)?;
        self.run_merge()?;
        self.run_profile_use(profile_use_args)
    }
}

fn collect_args<I, S>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect()
}

fn path_flag(prefix: &str, path: &Path) -> OsString {
    let mut flag = OsString::from(prefix);
    flag.push(path);
    flag
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("kfrc-pgo-test-{}-{sequence}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn args(command: &CommandSpec) -> Vec<String> {
        command
            .args()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn instrument_command_models_rustc_and_output_paths() {
        let pipeline = PgoPipeline::new("target/pgo", "target/pgo/training-bin")
            .with_tools("custom-rustc", "custom-profdata");

        assert_eq!(pipeline.raw_profile_dir(), Path::new("target/pgo/profraw"));
        assert_eq!(
            pipeline.raw_profile_pattern(),
            Path::new("target/pgo/profraw/default_%m_%p.profraw")
        );
        assert_eq!(
            pipeline.profdata_path(),
            Path::new("target/pgo/merged.profdata")
        );

        let command = pipeline.instrument_command(["main.rs", "-Copt-level=2"]);
        assert_eq!(command.program(), Path::new("custom-rustc"));
        assert_eq!(
            args(&command),
            [
                "main.rs",
                "-Copt-level=2",
                "-Cprofile-generate=target/pgo/profraw",
                "-o",
                "target/pgo/training-bin",
            ]
        );
        assert!(command.env().is_empty());
    }

    #[test]
    fn training_command_runs_instrumented_binary_with_raw_profile_pattern() {
        let pipeline = PgoPipeline::new("target/pgo", "target/pgo/training-bin");
        let command = pipeline.training_command(["--workload", "real"]);

        assert_eq!(command.program(), Path::new("target/pgo/training-bin"));
        assert_eq!(args(&command), ["--workload", "real"]);
        assert_eq!(
            command.env(),
            &[(
                OsString::from("LLVM_PROFILE_FILE"),
                OsString::from("target/pgo/profraw/default_%m_%p.profraw"),
            )]
        );
    }

    #[test]
    fn merge_discovers_sorted_raw_profiles_and_ignores_other_files() {
        let directory = TestDirectory::new();
        let pipeline = PgoPipeline::new(&directory.0, directory.0.join("training-bin"));
        let raw = pipeline.raw_profile_dir();
        fs::create_dir_all(&raw).unwrap();
        fs::write(raw.join("z.profraw"), b"z").unwrap();
        fs::write(raw.join("a.profraw"), b"a").unwrap();
        fs::write(raw.join("ignored.txt"), b"ignored").unwrap();
        fs::create_dir(raw.join("directory.profraw")).unwrap();

        let profiles = pipeline.discover_raw_profiles().unwrap();
        assert_eq!(profiles, [raw.join("a.profraw"), raw.join("z.profraw")]);

        let command = pipeline.merge_command(&profiles).unwrap();
        assert_eq!(command.program(), Path::new("llvm-profdata"));
        assert_eq!(
            args(&command),
            [
                "merge".to_owned(),
                format!("-output={}", pipeline.profdata_path().display()),
                raw.join("a.profraw").display().to_string(),
                raw.join("z.profraw").display().to_string(),
            ]
        );
    }

    #[test]
    fn merge_rejects_an_empty_raw_profile_list() {
        let pipeline = PgoPipeline::new("target/pgo", "training-bin");
        assert!(matches!(
            pipeline.merge_command(&[]),
            Err(PgoError::NoRawProfiles { .. })
        ));
    }

    #[test]
    fn profile_use_rejects_missing_empty_and_non_file_profiles() {
        let directory = TestDirectory::new();
        let pipeline = PgoPipeline::new(&directory.0, "training-bin");

        assert!(matches!(
            pipeline.profile_use_command(["main.rs"]),
            Err(PgoError::MissingProfdata { .. })
        ));

        fs::write(pipeline.profdata_path(), []).unwrap();
        assert!(matches!(
            pipeline.profile_use_command(["main.rs"]),
            Err(PgoError::EmptyProfdata { .. })
        ));

        fs::remove_file(pipeline.profdata_path()).unwrap();
        fs::create_dir(pipeline.profdata_path()).unwrap();
        assert!(matches!(
            pipeline.profile_use_command(["main.rs"]),
            Err(PgoError::ProfdataNotAFile { .. })
        ));
    }

    #[test]
    fn profile_use_requires_a_nonempty_profdata_and_builds_rustc_command() {
        let directory = TestDirectory::new();
        let pipeline = PgoPipeline::new(&directory.0, "training-bin");
        fs::write(pipeline.profdata_path(), b"nonempty profile").unwrap();

        let command = pipeline
            .profile_use_command(["main.rs", "-Copt-level=3"])
            .unwrap();
        assert_eq!(command.program(), Path::new("rustc"));
        assert_eq!(
            args(&command),
            [
                "main.rs",
                "-Copt-level=3",
                &format!("-Cprofile-use={}", pipeline.profdata_path().display()),
            ]
        );
    }

    #[test]
    fn merge_validates_the_generated_profdata() {
        let directory = TestDirectory::new();
        let pipeline = PgoPipeline::new(&directory.0, "training-bin").with_tools("rustc", "true");
        let raw = pipeline.raw_profile_dir();
        fs::create_dir_all(&raw).unwrap();
        fs::write(raw.join("one.profraw"), b"raw").unwrap();
        pipeline.run_merge().unwrap_err();
        assert!(matches!(
            pipeline.validate_profdata(),
            Err(PgoError::MissingProfdata { .. })
        ));
    }
}
