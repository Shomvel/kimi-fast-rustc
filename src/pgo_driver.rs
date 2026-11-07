//! Executable single-crate PGO workflow for the LLVM reference track.
//!
//! The native backend does not have LLVM profiles and must not call this module.
//! Each run gets a retained, unique work directory containing raw profiles, the
//! merged profile, intermediate binaries, and a `pgo.log` stage log. The requested
//! output is published only after every stage succeeds.

use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const PROFDATA_ENV: &str = "KFRC_LLVM_PROFDATA";
const RAW_PROFILE_DIRECTORY: &str = "profraw";
const MERGED_PROFILE: &str = "merged.profdata";
const PARTIAL_PROFILE: &str = "merged.profdata.partial";
const LOG_FILE: &str = "pgo.log";
const UNIQUE_PATH_ATTEMPTS: usize = 128;

static NEXT_UNIQUE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgoDriverStage {
    Prepare,
    Locate,
    Instrument,
    Train,
    Merge,
    ProfileUse,
    Publish,
}

impl fmt::Display for PgoDriverStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Prepare => "prepare",
            Self::Locate => "locate llvm-profdata",
            Self::Instrument => "instrument",
            Self::Train => "train",
            Self::Merge => "merge",
            Self::ProfileUse => "profile-use",
            Self::Publish => "publish",
        })
    }
}

#[derive(Debug)]
pub enum PgoDriverError {
    EmptyPath {
        kind: &'static str,
    },
    NoTrainingCommands,
    Io {
        stage: PgoDriverStage,
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Spawn {
        stage: PgoDriverStage,
        program: PathBuf,
        command: String,
        source: io::Error,
    },
    CommandFailed {
        stage: PgoDriverStage,
        program: PathBuf,
        command: String,
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    InvalidToolOutput {
        stage: PgoDriverStage,
        program: PathBuf,
        detail: String,
    },
    ProfdataNotFound {
        path: PathBuf,
    },
    NoRawProfiles {
        directory: PathBuf,
    },
    ArtifactMissing {
        stage: PgoDriverStage,
        path: PathBuf,
    },
    ArtifactNotAFile {
        stage: PgoDriverStage,
        path: PathBuf,
    },
    ArtifactEmpty {
        stage: PgoDriverStage,
        path: PathBuf,
    },
}

impl PgoDriverError {
    pub fn stage(&self) -> PgoDriverStage {
        match self {
            Self::EmptyPath { .. } | Self::NoTrainingCommands => PgoDriverStage::Prepare,
            Self::ProfdataNotFound { .. } => PgoDriverStage::Locate,
            Self::NoRawProfiles { .. } => PgoDriverStage::Merge,
            Self::Io { stage, .. }
            | Self::Spawn { stage, .. }
            | Self::CommandFailed { stage, .. }
            | Self::InvalidToolOutput { stage, .. }
            | Self::ArtifactMissing { stage, .. }
            | Self::ArtifactNotAFile { stage, .. }
            | Self::ArtifactEmpty { stage, .. } => *stage,
        }
    }

    pub fn status(&self) -> Option<ExitStatus> {
        match self {
            Self::CommandFailed { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn stdout(&self) -> Option<&[u8]> {
        match self {
            Self::CommandFailed { stdout, .. } => Some(stdout),
            _ => None,
        }
    }

    pub fn stderr(&self) -> Option<&[u8]> {
        match self {
            Self::CommandFailed { stderr, .. } => Some(stderr),
            _ => None,
        }
    }

    fn io(
        stage: PgoDriverStage,
        operation: &'static str,
        path: impl Into<PathBuf>,
        source: io::Error,
    ) -> Self {
        Self::Io {
            stage,
            operation,
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for PgoDriverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath { kind } => write!(formatter, "PGO {kind} path must not be empty"),
            Self::NoTrainingCommands => {
                formatter.write_str("PGO requires at least one training command")
            }
            Self::Io {
                stage,
                operation,
                path,
                source,
            } => write!(
                formatter,
                "PGO {stage} failed to {operation} {}: {source}",
                path.display()
            ),
            Self::Spawn {
                stage,
                program,
                command,
                source,
            } => write!(
                formatter,
                "PGO {stage} could not start {} ({command}): {source}",
                program.display()
            ),
            Self::CommandFailed {
                stage,
                command,
                status,
                stdout,
                stderr,
                ..
            } => {
                write!(
                    formatter,
                    "PGO {stage} command failed with {status}: {command}"
                )?;
                let stderr = String::from_utf8_lossy(stderr);
                let stderr = stderr.trim();
                if !stderr.is_empty() {
                    write!(formatter, ": {stderr}")?;
                } else {
                    let stdout = String::from_utf8_lossy(stdout);
                    let stdout = stdout.trim();
                    if !stdout.is_empty() {
                        write!(formatter, ": {stdout}")?;
                    }
                }
                Ok(())
            }
            Self::InvalidToolOutput {
                stage,
                program,
                detail,
            } => write!(
                formatter,
                "PGO {stage} received invalid output from {}: {detail}",
                program.display()
            ),
            Self::ProfdataNotFound { path } => write!(
                formatter,
                "LLVM llvm-profdata was not found at {}; set {PROFDATA_ENV} or install the rustc llvm-tools component",
                path.display()
            ),
            Self::NoRawProfiles { directory } => write!(
                formatter,
                "PGO training produced no nonempty .profraw files in {}",
                directory.display()
            ),
            Self::ArtifactMissing { stage, path } => write!(
                formatter,
                "PGO {stage} did not create expected file {}",
                path.display()
            ),
            Self::ArtifactNotAFile { stage, path } => write!(
                formatter,
                "PGO {stage} expected {} to be a regular file",
                path.display()
            ),
            Self::ArtifactEmpty { stage, path } => write!(
                formatter,
                "PGO {stage} produced an empty file {}",
                path.display()
            ),
        }
    }
}

impl Error for PgoDriverError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::Spawn { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingCommand {
    program: Option<PathBuf>,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    current_dir: Option<PathBuf>,
}

impl TrainingCommand {
    pub fn instrumented<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Self {
            program: None,
            args: collect_args(args),
            env: Vec::new(),
            current_dir: None,
        }
    }

    pub fn new<I, S>(program: impl Into<PathBuf>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Self {
            program: Some(program.into()),
            args: collect_args(args),
            env: Vec::new(),
            current_dir: None,
        }
    }

    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn with_current_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(directory.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct PgoRequest {
    source: PathBuf,
    output: PathBuf,
    rustc_args: Vec<OsString>,
    training_commands: Vec<TrainingCommand>,
}

impl PgoRequest {
    pub fn new(source: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            output: output.into(),
            rustc_args: Vec::new(),
            training_commands: Vec::new(),
        }
    }

    pub fn with_rustc_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.rustc_args = collect_args(args);
        self
    }

    pub fn with_training_commands<I>(mut self, commands: I) -> Self
    where
        I: IntoIterator<Item = TrainingCommand>,
    {
        self.training_commands = commands.into_iter().collect();
        self
    }

    pub fn push_training_command(&mut self, command: TrainingCommand) {
        self.training_commands.push(command);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgoOutcome {
    pub output: PathBuf,
    pub work_dir: PathBuf,
    pub raw_profiles: Vec<PathBuf>,
    pub profdata: PathBuf,
    pub log_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PgoDriver {
    rustc: PathBuf,
    llvm_profdata: Option<PathBuf>,
    temp_root: PathBuf,
}

impl PgoDriver {
    pub fn new() -> Self {
        let rustc = env::var_os("RUSTC")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("rustc"));
        Self {
            rustc,
            llvm_profdata: None,
            temp_root: env::temp_dir(),
        }
    }

    pub fn with_rustc(mut self, rustc: impl Into<PathBuf>) -> Self {
        self.rustc = rustc.into();
        self
    }

    pub fn with_llvm_profdata(mut self, llvm_profdata: impl Into<PathBuf>) -> Self {
        self.llvm_profdata = Some(llvm_profdata.into());
        self
    }

    pub fn with_tools(
        mut self,
        rustc: impl Into<PathBuf>,
        llvm_profdata: impl Into<PathBuf>,
    ) -> Self {
        self.rustc = rustc.into();
        self.llvm_profdata = Some(llvm_profdata.into());
        self
    }

    pub fn with_temp_root(mut self, temp_root: impl Into<PathBuf>) -> Self {
        self.temp_root = temp_root.into();
        self
    }

    pub fn locate_llvm_profdata(&self) -> Result<PathBuf, PgoDriverError> {
        let mut logger = Logger::silent();
        self.locate_llvm_profdata_logged(&mut logger)
    }

    fn locate_llvm_profdata_logged(&self, logger: &mut Logger) -> Result<PathBuf, PgoDriverError> {
        if let Some(path) = &self.llvm_profdata {
            logger.line(
                PgoDriverStage::Locate,
                format!("using configured llvm-profdata {}", path.display()),
            )?;
            return validate_tool(path);
        }
        locate_llvm_profdata_from(&self.rustc, env::var_os(PROFDATA_ENV), logger)
    }

    pub fn run(&self, request: &PgoRequest) -> Result<PgoOutcome, PgoDriverError> {
        validate_request(request)?;
        let work_dir = create_unique_directory(&self.temp_root)?;
        let work_dir = fs::canonicalize(&work_dir).map_err(|source| {
            PgoDriverError::io(
                PgoDriverStage::Prepare,
                "canonicalize PGO work directory",
                &work_dir,
                source,
            )
        })?;
        let log_path = work_dir.join(LOG_FILE);
        let mut logger = Logger::create(&log_path)?;
        logger.line(
            PgoDriverStage::Prepare,
            format!("work directory: {}", work_dir.display()),
        )?;

        let result = self.run_in_work_dir(request, &work_dir, &log_path, &mut logger);
        match &result {
            Ok(outcome) => {
                let _ = logger.line(
                    PgoDriverStage::Publish,
                    format!("success: {}", outcome.output.display()),
                );
            }
            Err(error) => {
                let _ = logger.line(error.stage(), format!("failed: {error}"));
            }
        }
        result
    }

    fn run_in_work_dir(
        &self,
        request: &PgoRequest,
        work_dir: &Path,
        log_path: &Path,
        logger: &mut Logger,
    ) -> Result<PgoOutcome, PgoDriverError> {
        let raw_dir = work_dir.join(RAW_PROFILE_DIRECTORY);
        fs::create_dir(&raw_dir).map_err(|source| {
            PgoDriverError::io(
                PgoDriverStage::Prepare,
                "create raw profile directory",
                &raw_dir,
                source,
            )
        })?;

        let llvm_profdata = self.locate_llvm_profdata_logged(logger)?;
        let instrumented = work_dir.join(executable_name("instrumented"));
        let mut instrument_args = request.rustc_args.clone();
        instrument_args.push(request.source.as_os_str().to_owned());
        instrument_args.push(path_flag("-Cprofile-generate=", &raw_dir));
        instrument_args.push(OsString::from("-o"));
        instrument_args.push(instrumented.as_os_str().to_owned());
        execute(
            PgoDriverStage::Instrument,
            &self.rustc,
            &instrument_args,
            &[],
            None,
            logger,
        )?;
        validate_artifact(PgoDriverStage::Instrument, &instrumented)?;

        for (index, training) in request.training_commands.iter().enumerate() {
            let program = training.program.as_deref().unwrap_or(&instrumented);
            let profile_pattern = raw_dir.join(format!("train_{index}_%m_%p.profraw"));
            let profile_env = vec![(
                OsString::from("LLVM_PROFILE_FILE"),
                profile_pattern.as_os_str().to_owned(),
            )];
            logger.line(
                PgoDriverStage::Train,
                format!(
                    "training command {} of {}; LLVM_PROFILE_FILE={}",
                    index + 1,
                    request.training_commands.len(),
                    profile_pattern.display()
                ),
            )?;
            let mut env = training.env.clone();
            env.extend(profile_env);
            execute(
                PgoDriverStage::Train,
                program,
                &training.args,
                &env,
                training.current_dir.as_deref(),
                logger,
            )?;
        }

        let raw_profiles = discover_raw_profiles(&raw_dir)?;
        logger.line(
            PgoDriverStage::Merge,
            format!("merging {} nonempty raw profiles", raw_profiles.len()),
        )?;
        let merged = work_dir.join(MERGED_PROFILE);
        let partial = work_dir.join(PARTIAL_PROFILE);
        let mut merge_args = vec![OsString::from("merge"), path_flag("-output=", &partial)];
        merge_args.extend(raw_profiles.iter().map(|path| path.as_os_str().to_owned()));
        execute(
            PgoDriverStage::Merge,
            &llvm_profdata,
            &merge_args,
            &[],
            None,
            logger,
        )?;
        validate_artifact(PgoDriverStage::Merge, &partial)?;
        fs::rename(&partial, &merged).map_err(|source| {
            PgoDriverError::io(
                PgoDriverStage::Merge,
                "publish merged profile",
                &merged,
                source,
            )
        })?;

        let final_candidate = work_dir.join(executable_name("final"));
        let mut profile_use_args = request.rustc_args.clone();
        profile_use_args.push(request.source.as_os_str().to_owned());
        profile_use_args.push(path_flag("-Cprofile-use=", &merged));
        profile_use_args.push(OsString::from("-o"));
        profile_use_args.push(final_candidate.as_os_str().to_owned());
        execute(
            PgoDriverStage::ProfileUse,
            &self.rustc,
            &profile_use_args,
            &[],
            None,
            logger,
        )?;
        validate_artifact(PgoDriverStage::ProfileUse, &final_candidate)?;
        publish_output(&final_candidate, &request.output)?;

        Ok(PgoOutcome {
            output: request.output.clone(),
            work_dir: work_dir.to_path_buf(),
            raw_profiles,
            profdata: merged,
            log_path: log_path.to_path_buf(),
        })
    }
}

impl Default for PgoDriver {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_request(request: &PgoRequest) -> Result<(), PgoDriverError> {
    if request.source.as_os_str().is_empty() {
        return Err(PgoDriverError::EmptyPath { kind: "source" });
    }
    if request.output.as_os_str().is_empty() || request.output.file_name().is_none() {
        return Err(PgoDriverError::EmptyPath { kind: "output" });
    }
    if request.training_commands.is_empty() {
        return Err(PgoDriverError::NoTrainingCommands);
    }
    Ok(())
}

fn locate_llvm_profdata_from(
    rustc: &Path,
    override_path: Option<OsString>,
    logger: &mut Logger,
) -> Result<PathBuf, PgoDriverError> {
    if let Some(path) = override_path.filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        logger.line(
            PgoDriverStage::Locate,
            format!("using {PROFDATA_ENV}={}", path.display()),
        )?;
        return validate_tool(&path);
    }

    let sysroot_output = execute(
        PgoDriverStage::Locate,
        rustc,
        &[OsString::from("--print"), OsString::from("sysroot")],
        &[],
        None,
        logger,
    )?;
    let sysroot = tool_stdout(rustc, &sysroot_output, "sysroot")?;
    let version_output = execute(
        PgoDriverStage::Locate,
        rustc,
        &[OsString::from("-vV")],
        &[],
        None,
        logger,
    )?;
    let version = tool_stdout(rustc, &version_output, "version")?;
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .ok_or_else(|| PgoDriverError::InvalidToolOutput {
            stage: PgoDriverStage::Locate,
            program: rustc.to_path_buf(),
            detail: "rustc -vV did not report a host triple".to_owned(),
        })?;
    let path = PathBuf::from(sysroot.trim())
        .join("lib")
        .join("rustlib")
        .join(host)
        .join("bin")
        .join(executable_name("llvm-profdata"));
    logger.line(
        PgoDriverStage::Locate,
        format!("trying rustc llvm-tools path {}", path.display()),
    )?;
    validate_tool(&path)?;
    Ok(path)
}

fn tool_stdout(rustc: &Path, output: &Output, description: &str) -> Result<String, PgoDriverError> {
    String::from_utf8(output.stdout.clone()).map_err(|source| PgoDriverError::InvalidToolOutput {
        stage: PgoDriverStage::Locate,
        program: rustc.to_path_buf(),
        detail: format!("{description} output is not UTF-8: {source}"),
    })
}

fn validate_tool(path: &Path) -> Result<PathBuf, PgoDriverError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() > 0 => Ok(path.to_path_buf()),
        Ok(metadata) if metadata.is_file() => Err(PgoDriverError::ArtifactEmpty {
            stage: PgoDriverStage::Locate,
            path: path.to_path_buf(),
        }),
        Ok(_) => Err(PgoDriverError::ArtifactNotAFile {
            stage: PgoDriverStage::Locate,
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            Err(PgoDriverError::ProfdataNotFound {
                path: path.to_path_buf(),
            })
        }
        Err(source) => Err(PgoDriverError::io(
            PgoDriverStage::Locate,
            "inspect llvm-profdata",
            path,
            source,
        )),
    }
}

fn validate_artifact(stage: PgoDriverStage, path: &Path) -> Result<(), PgoDriverError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(PgoDriverError::ArtifactMissing {
                stage,
                path: path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(PgoDriverError::io(
                stage,
                "inspect generated artifact",
                path,
                source,
            ));
        }
    };
    if !metadata.is_file() {
        Err(PgoDriverError::ArtifactNotAFile {
            stage,
            path: path.to_path_buf(),
        })
    } else if metadata.len() == 0 {
        Err(PgoDriverError::ArtifactEmpty {
            stage,
            path: path.to_path_buf(),
        })
    } else {
        Ok(())
    }
}

fn discover_raw_profiles(directory: &Path) -> Result<Vec<PathBuf>, PgoDriverError> {
    let entries = fs::read_dir(directory).map_err(|source| {
        PgoDriverError::io(
            PgoDriverStage::Merge,
            "read raw profile directory",
            directory,
            source,
        )
    })?;
    let mut profiles = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| {
            PgoDriverError::io(
                PgoDriverStage::Merge,
                "read raw profile directory entry",
                directory,
                source,
            )
        })?;
        let path = entry.path();
        if path.extension() != Some(OsStr::new("profraw")) {
            continue;
        }
        let metadata = entry.metadata().map_err(|source| {
            PgoDriverError::io(PgoDriverStage::Merge, "inspect raw profile", &path, source)
        })?;
        if metadata.is_file() && metadata.len() > 0 {
            profiles.push(path);
        }
    }
    profiles.sort();
    if profiles.is_empty() {
        Err(PgoDriverError::NoRawProfiles {
            directory: directory.to_path_buf(),
        })
    } else {
        Ok(profiles)
    }
}

fn execute(
    stage: PgoDriverStage,
    program: &Path,
    args: &[OsString],
    env: &[(OsString, OsString)],
    current_dir: Option<&Path>,
    logger: &mut Logger,
) -> Result<Output, PgoDriverError> {
    let rendered = render_command(program, args);
    logger.line(stage, format!("running {rendered}"))?;
    let mut command = Command::new(program);
    command.args(args).envs(env.iter().cloned());
    if let Some(directory) = current_dir {
        command.current_dir(directory);
    }
    let output = command.output().map_err(|source| PgoDriverError::Spawn {
        stage,
        program: program.to_path_buf(),
        command: rendered.clone(),
        source,
    })?;
    if !output.status.success() {
        let error = PgoDriverError::CommandFailed {
            stage,
            program: program.to_path_buf(),
            command: rendered,
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        };
        let _ = logger.line(stage, format!("command failed: {error}"));
        return Err(error);
    }
    logger.line(stage, "command succeeded".to_owned())?;
    logger.output(stage, "stdout", &output.stdout)?;
    logger.output(stage, "stderr", &output.stderr)?;
    Ok(output)
}

fn render_command(program: &Path, args: &[OsString]) -> String {
    let mut rendered = format!("{:?}", program.as_os_str());
    for arg in args {
        rendered.push(' ');
        rendered.push_str(&format!("{arg:?}"));
    }
    rendered
}

struct Logger {
    file: Option<File>,
    path: PathBuf,
    echo: bool,
}

impl Logger {
    fn silent() -> Self {
        Self {
            file: None,
            path: PathBuf::new(),
            echo: false,
        }
    }

    fn create(path: &Path) -> Result<Self, PgoDriverError> {
        let file = File::create(path).map_err(|source| {
            PgoDriverError::io(PgoDriverStage::Prepare, "create PGO log", path, source)
        })?;
        Ok(Self {
            file: Some(file),
            path: path.to_path_buf(),
            echo: true,
        })
    }

    fn line(&mut self, stage: PgoDriverStage, message: String) -> Result<(), PgoDriverError> {
        let line = format!("[{stage}] {message}");
        eprintln_conditional(self.echo, &line);
        if let Some(file) = &mut self.file {
            writeln!(file, "{line}")
                .map_err(|source| PgoDriverError::io(stage, "write PGO log", &self.path, source))?;
            file.flush()
                .map_err(|source| PgoDriverError::io(stage, "flush PGO log", &self.path, source))?;
        }
        Ok(())
    }

    fn output(
        &mut self,
        stage: PgoDriverStage,
        stream: &str,
        bytes: &[u8],
    ) -> Result<(), PgoDriverError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(bytes);
        for line in text.trim_end().lines() {
            self.line(stage, format!("{stream}: {line}"))?;
        }
        Ok(())
    }
}

fn eprintln_conditional(enabled: bool, line: &str) {
    if enabled {
        eprintln!("kfrc-pgo {line}");
    }
}

fn create_unique_directory(root: &Path) -> Result<PathBuf, PgoDriverError> {
    fs::create_dir_all(root).map_err(|source| {
        PgoDriverError::io(
            PgoDriverStage::Prepare,
            "create PGO temporary root",
            root,
            source,
        )
    })?;
    for _ in 0..UNIQUE_PATH_ATTEMPTS {
        let path = root.join(unique_name("kfrc-pgo"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(PgoDriverError::io(
                    PgoDriverStage::Prepare,
                    "create unique PGO work directory",
                    path,
                    source,
                ));
            }
        }
    }
    Err(PgoDriverError::io(
        PgoDriverStage::Prepare,
        "create unique PGO work directory in",
        root,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many temporary path collisions",
        ),
    ))
}

fn publish_output(candidate: &Path, output: &Path) -> Result<(), PgoDriverError> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| {
        PgoDriverError::io(
            PgoDriverStage::Publish,
            "create output directory",
            parent,
            source,
        )
    })?;
    let file_name = output
        .file_name()
        .expect("request validation checked the output file name")
        .to_string_lossy();

    for _ in 0..UNIQUE_PATH_ATTEMPTS {
        let staging = parent.join(unique_name(&format!(".{file_name}.kfrc-pgo")));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
        {
            Ok(file) => {
                drop(file);
                if let Err(source) = fs::copy(candidate, &staging) {
                    let _ = fs::remove_file(&staging);
                    return Err(PgoDriverError::io(
                        PgoDriverStage::Publish,
                        "copy final executable to staging file",
                        &staging,
                        source,
                    ));
                }
                if let Err(source) = fs::rename(&staging, output) {
                    let _ = fs::remove_file(&staging);
                    return Err(PgoDriverError::io(
                        PgoDriverStage::Publish,
                        "publish final executable",
                        output,
                        source,
                    ));
                }
                return Ok(());
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(PgoDriverError::io(
                    PgoDriverStage::Publish,
                    "create output staging file",
                    staging,
                    source,
                ));
            }
        }
    }
    Err(PgoDriverError::io(
        PgoDriverStage::Publish,
        "create unique output staging file in",
        parent,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many temporary path collisions",
        ),
    ))
}

fn unique_name(prefix: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let id = NEXT_UNIQUE_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{timestamp:x}-{id:x}", std::process::id())
}

fn executable_name(stem: &str) -> String {
    format!("{stem}{}", env::consts::EXE_SUFFIX)
}

fn path_flag(prefix: &str, path: &Path) -> OsString {
    let mut flag = OsString::from(prefix);
    flag.push(path);
    flag
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            Self(create_unique_directory(&env::temp_dir()).unwrap())
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, contents).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    fn shell_quote(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
    }

    #[cfg(unix)]
    fn mock_rustc(path: &Path, log: &Path, fail_final: bool) {
        let log = shell_quote(log);
        write_executable(
            path,
            &format!(
                r#"#!/bin/sh
set -eu
printf 'rustc' >> {log}
for arg in "$@"; do printf ' <%s>' "$arg" >> {log}; done
printf '\n' >> {log}
mode=final
out=
fail_final=
for arg in "$@"; do
    case "$arg" in
        -Cprofile-generate=*) mode=instrument ;;
        -Cprofile-use=*) mode=final; profile=${{arg#-Cprofile-use=}} ;;
        fail-final) fail_final=yes ;;
    esac
done
while [ "$#" -gt 0 ]; do
    if [ "$1" = -o ]; then shift; out=$1; fi
    shift
done
test -n "$out"
if [ "$mode" = instrument ]; then
    cat > "$out" <<'TRAINER'
#!/bin/sh
if [ "${{1:-}}" = fail ]; then
    echo training-stdout
    echo training-stderr >&2
    exit 17
fi
profile=$(printf '%s' "$LLVM_PROFILE_FILE" | sed "s/%m_%p/$1/")
printf 'raw-%s\n' "$1" > "$profile"
printf 'trainer <%s> <%s>\n' "$1" "$LLVM_PROFILE_FILE" >> TRAINER_LOG
TRAINER
    sed -i.bak "s|TRAINER_LOG|{log}|g" "$out"
    rm "$out.bak"
else
    test -s "$profile"
    if [ "{fail_final}" = yes ]; then
        echo final-compile-failed >&2
        exit 19
    fi
    printf '#!/bin/sh\nexit 0\n' > "$out"
fi
chmod +x "$out"
"#,
                log = log,
                fail_final = if fail_final { "yes" } else { "" },
            ),
        );
    }

    #[cfg(unix)]
    fn mock_profdata(path: &Path, log: &Path, empty: bool) {
        let log = shell_quote(log);
        write_executable(
            path,
            &format!(
                r#"#!/bin/sh
set -eu
printf 'profdata' >> {log}
for arg in "$@"; do printf ' <%s>' "$arg" >> {log}; done
printf '\n' >> {log}
out=
inputs=
for arg in "$@"; do
    case "$arg" in
        -output=*) out=${{arg#-output=}} ;;
        merge) ;;
        *) inputs="$inputs $arg" ;;
    esac
done
test -n "$out"
: > "$out"
if [ "{empty}" = yes ]; then exit 0; fi
for input in $inputs; do
    test -s "$input"
    cat "$input" >> "$out"
done
"#,
                log = log,
                empty = if empty { "yes" } else { "" },
            ),
        );
    }

    #[cfg(unix)]
    fn successful_mock_workflow() -> (TestDirectory, PgoOutcome) {
        let root = TestDirectory::new();
        let rustc = root.0.join("mock-rustc");
        let profdata = root.0.join("mock-profdata");
        let commands = root.0.join("commands.log");
        mock_rustc(&rustc, &commands, false);
        mock_profdata(&profdata, &commands, false);
        let source = root.0.join("main.rs");
        let output = root.0.join("optimized");
        fs::write(&source, "fn main() {}\n").unwrap();
        let request = PgoRequest::new(&source, &output)
            .with_rustc_args(["-Copt-level=2"])
            .with_training_commands([
                TrainingCommand::instrumented(["one"]),
                TrainingCommand::instrumented(["two"]),
            ]);
        let outcome = PgoDriver::new()
            .with_tools(&rustc, &profdata)
            .with_temp_root(&root.0)
            .run(&request)
            .unwrap();
        (root, outcome)
    }

    #[test]
    fn rejects_an_empty_training_set_before_creating_a_work_directory() {
        let root = TestDirectory::new();
        let temp_root = root.0.join("not-created");
        let request = PgoRequest::new("main.rs", root.0.join("output"));
        let error = PgoDriver::new()
            .with_temp_root(&temp_root)
            .run(&request)
            .unwrap_err();
        assert!(matches!(error, PgoDriverError::NoTrainingCommands));
        assert!(!temp_root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn mock_tools_run_the_complete_multi_training_workflow() {
        let (_root, outcome) = successful_mock_workflow();
        assert_eq!(outcome.raw_profiles.len(), 2);
        assert!(
            outcome
                .raw_profiles
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(fs::metadata(&outcome.profdata).unwrap().len() > 0);
        assert!(Command::new(&outcome.output).status().unwrap().success());
        let log = fs::read_to_string(&outcome.log_path).unwrap();
        for stage in [
            "[locate llvm-profdata]",
            "[instrument]",
            "[train]",
            "[merge]",
            "[profile-use]",
            "[publish] success:",
        ] {
            assert!(log.contains(stage), "missing {stage:?} in log:\n{log}");
        }
        let commands =
            fs::read_to_string(outcome.work_dir.parent().unwrap().join("commands.log")).unwrap();
        assert_eq!(commands.matches("trainer <").count(), 2);
        assert!(commands.contains("train_0_"));
        assert!(commands.contains("train_1_"));
        assert!(commands.contains("-Cprofile-use="));
    }

    #[cfg(unix)]
    #[test]
    fn training_failure_preserves_diagnostics_and_creates_no_final_artifacts() {
        let root = TestDirectory::new();
        let rustc = root.0.join("mock-rustc");
        let profdata = root.0.join("mock-profdata");
        let commands = root.0.join("commands.log");
        mock_rustc(&rustc, &commands, false);
        mock_profdata(&profdata, &commands, false);
        let output = root.0.join("optimized");
        let request = PgoRequest::new("main.rs", &output)
            .with_training_commands([TrainingCommand::instrumented(["fail"])]);
        let error = PgoDriver::new()
            .with_tools(&rustc, &profdata)
            .with_temp_root(&root.0)
            .run(&request)
            .unwrap_err();
        assert_eq!(error.stage(), PgoDriverStage::Train);
        assert_eq!(error.status().and_then(|status| status.code()), Some(17));
        assert_eq!(error.stdout(), Some(&b"training-stdout\n"[..]));
        assert_eq!(error.stderr(), Some(&b"training-stderr\n"[..]));
        assert!(!output.exists());
    }

    #[cfg(unix)]
    #[test]
    fn empty_merged_profile_is_an_error_and_does_not_run_profile_use() {
        let root = TestDirectory::new();
        let rustc = root.0.join("mock-rustc");
        let profdata = root.0.join("mock-profdata");
        let commands = root.0.join("commands.log");
        mock_rustc(&rustc, &commands, false);
        mock_profdata(&profdata, &commands, true);
        let output = root.0.join("optimized");
        let request = PgoRequest::new("main.rs", &output)
            .with_training_commands([TrainingCommand::instrumented(["one"])]);
        let error = PgoDriver::new()
            .with_tools(&rustc, &profdata)
            .with_temp_root(&root.0)
            .run(&request)
            .unwrap_err();
        assert!(matches!(
            error,
            PgoDriverError::ArtifactEmpty {
                stage: PgoDriverStage::Merge,
                ..
            }
        ));
        assert!(!output.exists());
        let commands = fs::read_to_string(commands).unwrap();
        assert!(!commands.contains("-Cprofile-use="));
    }

    #[cfg(unix)]
    #[test]
    fn failed_final_compilation_never_publishes_the_requested_output() {
        let root = TestDirectory::new();
        let rustc = root.0.join("mock-rustc");
        let profdata = root.0.join("mock-profdata");
        let commands = root.0.join("commands.log");
        mock_rustc(&rustc, &commands, true);
        mock_profdata(&profdata, &commands, false);
        let output = root.0.join("optimized");
        let request = PgoRequest::new("main.rs", &output)
            .with_training_commands([TrainingCommand::instrumented(["one"])]);
        let error = PgoDriver::new()
            .with_tools(&rustc, &profdata)
            .with_temp_root(&root.0)
            .run(&request)
            .unwrap_err();
        assert_eq!(error.stage(), PgoDriverStage::ProfileUse);
        assert_eq!(error.status().and_then(|status| status.code()), Some(19));
        assert!(!output.exists());
    }

    #[cfg(unix)]
    #[test]
    fn locates_profdata_from_rustc_sysroot_and_host() {
        let root = TestDirectory::new();
        let sysroot = root.0.join("sysroot");
        let host = "mock-host-triple";
        let tool = sysroot
            .join("lib")
            .join("rustlib")
            .join(host)
            .join("bin")
            .join(executable_name("llvm-profdata"));
        fs::create_dir_all(tool.parent().unwrap()).unwrap();
        fs::write(&tool, "mock llvm-profdata").unwrap();
        let rustc = root.0.join("rustc");
        write_executable(
            &rustc,
            &format!(
                "#!/bin/sh\nif [ \"${{1:-}}\" = --print ]; then\n  printf '%s\\n' '{}'\nelse\n  printf 'rustc mock\\nhost: {}\\nrelease: mock\\n'\nfi\n",
                sysroot.display(),
                host
            ),
        );

        let mut logger = Logger::silent();
        let located = locate_llvm_profdata_from(&rustc, None, &mut logger).unwrap();
        assert_eq!(located, tool);

        let override_tool = root.0.join("override-profdata");
        fs::write(&override_tool, "override").unwrap();
        let located = locate_llvm_profdata_from(
            &root.0.join("must-not-run"),
            Some(override_tool.as_os_str().to_owned()),
            &mut logger,
        )
        .unwrap();
        assert_eq!(located, override_tool);
    }

    #[test]
    fn real_rustc_pgo_probe_when_llvm_tools_are_available() {
        let driver = PgoDriver::new();
        if let Err(error) = driver.locate_llvm_profdata() {
            eprintln!("skipping real rustc PGO probe: {error}");
            return;
        }

        let root = TestDirectory::new();
        let source = root.0.join("probe.rs");
        let output = root.0.join(executable_name("probe"));
        fs::write(
            &source,
            r#"fn main() {
    let input: u64 = std::env::args()
        .skip(1)
        .filter_map(|arg| arg.parse::<u64>().ok())
        .sum();
    let mut value = 0;
    for index in 0..10_000 {
        if (index + input) % 3 == 0 {
            value += index;
        } else {
            value ^= index + 1;
        }
    }
    std::process::exit(if value == u64::MAX { 1 } else { 0 });
}
"#,
        )
        .unwrap();
        let request = PgoRequest::new(&source, &output)
            .with_rustc_args(["-Copt-level=2"])
            .with_training_commands([
                TrainingCommand::instrumented(["1"]),
                TrainingCommand::instrumented(["2"]),
            ]);
        let outcome = driver.with_temp_root(&root.0).run(&request).unwrap();
        assert_eq!(outcome.raw_profiles.len(), 2);
        assert!(fs::metadata(&outcome.profdata).unwrap().len() > 0);
        assert!(Command::new(&outcome.output).status().unwrap().success());
        assert!(
            fs::read_to_string(&outcome.log_path)
                .unwrap()
                .contains("[publish] success:")
        );
    }
}
