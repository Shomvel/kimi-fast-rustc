//! Real Cargo builds and runtime measurements for LLVM tuning candidates.
//!
//! Every execution uses a fresh target directory. Commands, toolchain output,
//! build logs, artifact metadata, and benchmark samples are retained in the
//! returned report. Only completed measurements are passed to `tuning::search`.

use crate::artifact::{self, ArtifactMetadata};
use crate::benchmark::{self, BenchmarkConfig, BenchmarkResult, Redirect};
use crate::llvm::LlvmConfig;
use crate::rustc;
use crate::tuning::{
    self, CandidateMeasurement, Measurement, MeasurementError, PolicyError, SearchPolicy,
    SearchReport,
};
use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_MAX_CANDIDATES: usize = 4;
pub const MIN_RUNTIME_SAMPLES: usize = tuning::DEFAULT_MIN_SAMPLES;

const UNIQUE_DIRECTORY_ATTEMPTS: usize = 128;
const FORBIDDEN_FAST_MATH: &[&str] = &[
    "fast-math",
    "fast_math",
    "unsafe-fp",
    "reassoc",
    "nnan",
    "ninf",
    "finite-math-only",
    "no-signed-zeros",
    "no-trapping-math",
];

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, Option<OsString>)>,
    pub current_dir: PathBuf,
}

impl CommandSpec {
    pub fn render(&self) -> String {
        let mut rendered = String::new();
        for (key, value) in &self.env {
            match value {
                Some(value) => {
                    rendered.push_str(&format!("{:?}={:?} ", key, value));
                }
                None => {
                    rendered.push_str(&format!("env -u {:?} ", key));
                }
            }
        }
        rendered.push_str(&format!("{:?}", self.program.as_os_str()));
        for arg in &self.args {
            rendered.push_str(&format!(" {arg:?}"));
        }
        rendered.push_str(&format!(" (cwd {:?})", self.current_dir.as_os_str()));
        rendered
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandRecord {
    pub command: CommandSpec,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandRecord {
    fn new(command: CommandSpec, output: Output) -> Self {
        Self {
            command,
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RustcVersion {
    pub record: CommandRecord,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TuningExecution {
    pub target_dir: PathBuf,
    pub artifact_path: PathBuf,
    pub build_command: CommandSpec,
    pub build_record: Option<CommandRecord>,
    pub artifact: Option<ArtifactMetadata>,
    pub benchmark_command: Option<CommandSpec>,
    pub benchmark: Option<BenchmarkResult>,
    pub error: Option<MeasurementError>,
}

impl TuningExecution {
    fn new(target_dir: PathBuf, artifact_path: PathBuf, build_command: CommandSpec) -> Self {
        Self {
            target_dir,
            artifact_path,
            build_command,
            build_record: None,
            artifact: None,
            benchmark_command: None,
            benchmark: None,
            error: None,
        }
    }

    pub fn measurement(&self) -> Result<Measurement, MeasurementError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let artifact = self.artifact.as_ref().ok_or_else(|| {
            MeasurementError::new("artifact metadata is absent; no measurement is available")
        })?;
        let benchmark = self.benchmark.as_ref().ok_or_else(|| {
            MeasurementError::new("benchmark result is absent; no measurement is available")
        })?;
        Ok(Measurement::new(
            benchmark.samples.clone(),
            artifact.size_bytes,
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TuningRunReport {
    pub run_dir: PathBuf,
    pub configuration: TuningRunner,
    pub rustc_version: RustcVersion,
    pub baseline: TuningExecution,
    pub candidates: Vec<TuningExecution>,
    pub omitted_candidate_count: usize,
    pub search: SearchReport,
}

impl TuningRunReport {
    pub fn has_measurement_failures(&self) -> bool {
        self.baseline.error.is_some()
            || self
                .candidates
                .iter()
                .any(|candidate| candidate.error.is_some())
    }

    pub fn to_json(&self) -> String {
        tuning_run_report_json(self)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TuningRunner {
    pub manifest_path: PathBuf,
    pub target_root: PathBuf,
    pub artifact_relative_path: PathBuf,
    pub cargo: PathBuf,
    pub rustc: PathBuf,
    pub cargo_args: Vec<OsString>,
    pub workload_program: Option<PathBuf>,
    pub benchmark_args: Vec<String>,
    pub benchmark_stdin: Redirect,
    pub benchmark_stdout: Redirect,
    pub runs: usize,
    pub warmup_runs: usize,
    pub expected_exit_code: i32,
    pub max_candidates: usize,
    pub policy: SearchPolicy,
}

impl TuningRunner {
    pub fn new(
        manifest_path: impl Into<PathBuf>,
        target_root: impl Into<PathBuf>,
        artifact_relative_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            manifest_path: manifest_path.into(),
            target_root: target_root.into(),
            artifact_relative_path: artifact_relative_path.into(),
            cargo: PathBuf::from("cargo"),
            rustc: rustc::rustc_from_env(),
            cargo_args: Vec::new(),
            workload_program: None,
            benchmark_args: Vec::new(),
            benchmark_stdin: Redirect::Null,
            benchmark_stdout: Redirect::Null,
            runs: MIN_RUNTIME_SAMPLES,
            warmup_runs: 0,
            expected_exit_code: 0,
            max_candidates: DEFAULT_MAX_CANDIDATES,
            policy: SearchPolicy::default(),
        }
    }

    pub fn run(&self, candidates: &[LlvmConfig]) -> Result<TuningRunReport, TuningRunnerError> {
        self.validate()?;
        let current_dir = env::current_dir().map_err(|source| TuningRunnerError::Io {
            operation: "determine the current directory",
            path: PathBuf::from("."),
            source,
        })?;
        let manifest_path = absolute_path(&current_dir, &self.manifest_path);
        let target_root = absolute_path(&current_dir, &self.target_root);
        let rustc_version = self.rustc_version(&current_dir)?;
        let run_dir = create_run_directory(&target_root)?;

        let baseline_dir = run_dir.join("baseline");
        let baseline_command =
            self.build_command(&current_dir, &manifest_path, &baseline_dir, Vec::new());
        let mut baseline = TuningExecution::new(
            baseline_dir.clone(),
            baseline_dir.join(&self.artifact_relative_path),
            baseline_command,
        );
        self.measure(&mut baseline, &current_dir, &rustc_version.version);

        let candidate_count = candidates.len().min(self.max_candidates);
        let omitted_candidate_count = candidates.len() - candidate_count;
        let mut executions = Vec::with_capacity(candidate_count);
        let mut measurements = Vec::with_capacity(candidate_count);

        for (index, config) in candidates[..candidate_count].iter().copied().enumerate() {
            let target_dir = run_dir.join(format!("candidate-{index:03}"));
            let artifact_path = target_dir.join(&self.artifact_relative_path);
            let rustc_args = config.rustc_args();
            let build_command = self.build_command(
                &current_dir,
                &manifest_path,
                &target_dir,
                rustc_args.clone().unwrap_or_default(),
            );
            let mut execution = TuningExecution::new(target_dir, artifact_path, build_command);

            match rustc_args {
                Ok(args) => {
                    if let Some(argument) = forbidden_fast_math(args.iter()) {
                        execution.error = Some(MeasurementError::new(format!(
                            "candidate contains forbidden fast-math argument {argument:?}"
                        )));
                    } else {
                        self.measure(&mut execution, &current_dir, &rustc_version.version);
                    }
                }
                Err(error) => {
                    execution.error = Some(MeasurementError::new(format!(
                        "invalid LLVM configuration: {error}"
                    )));
                }
            }

            let measurement = execution.measurement();
            measurements.push(match measurement {
                Ok(measurement) => CandidateMeasurement::new(config, measurement),
                Err(error) => CandidateMeasurement::failed(config, error),
            });
            executions.push(execution);
        }

        let search = tuning::search(baseline.measurement(), &measurements, &self.policy)
            .map_err(TuningRunnerError::Policy)?;
        Ok(TuningRunReport {
            run_dir,
            configuration: self.clone(),
            rustc_version,
            baseline,
            candidates: executions,
            omitted_candidate_count,
            search,
        })
    }

    pub fn run_default(&self) -> Result<TuningRunReport, TuningRunnerError> {
        self.run(&tuning::default_candidate_configs())
    }

    fn validate(&self) -> Result<(), TuningRunnerError> {
        for (name, path) in [
            ("manifest path", &self.manifest_path),
            ("target root", &self.target_root),
            ("artifact relative path", &self.artifact_relative_path),
            ("cargo executable", &self.cargo),
            ("rustc executable", &self.rustc),
        ] {
            if path.as_os_str().is_empty() {
                return Err(TuningRunnerError::InvalidConfig(format!(
                    "{name} must not be empty"
                )));
            }
        }
        if self
            .workload_program
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(TuningRunnerError::InvalidConfig(
                "workload program must not be empty".to_owned(),
            ));
        }
        if self.artifact_relative_path.is_absolute()
            || self
                .artifact_relative_path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(TuningRunnerError::InvalidConfig(format!(
                "artifact path must stay relative to each target directory: {}",
                self.artifact_relative_path.display()
            )));
        }
        if self.runs < MIN_RUNTIME_SAMPLES {
            return Err(TuningRunnerError::InvalidConfig(format!(
                "runtime measurement requires at least {MIN_RUNTIME_SAMPLES} runs (got {})",
                self.runs
            )));
        }
        if self.policy.min_samples < MIN_RUNTIME_SAMPLES {
            return Err(TuningRunnerError::InvalidConfig(format!(
                "tuning policy requires at least {MIN_RUNTIME_SAMPLES} samples (got {})",
                self.policy.min_samples
            )));
        }
        if self.runs < self.policy.min_samples {
            return Err(TuningRunnerError::InvalidConfig(format!(
                "configured runs ({}) are fewer than policy minimum samples ({})",
                self.runs, self.policy.min_samples
            )));
        }
        if self.max_candidates == 0 {
            return Err(TuningRunnerError::InvalidConfig(
                "maximum candidate count must be greater than zero".to_owned(),
            ));
        }
        validate_policy(&self.policy)?;
        if let Some(argument) = forbidden_fast_math(self.cargo_args.iter()) {
            return Err(TuningRunnerError::InvalidConfig(format!(
                "Cargo arguments contain forbidden fast-math option {argument:?}"
            )));
        }
        Ok(())
    }

    fn rustc_version(&self, current_dir: &Path) -> Result<RustcVersion, TuningRunnerError> {
        let command = CommandSpec {
            program: self.rustc.clone(),
            args: vec![OsString::from("--version"), OsString::from("--verbose")],
            env: Vec::new(),
            current_dir: current_dir.to_path_buf(),
        };
        let output = execute(&command).map_err(|source| TuningRunnerError::VersionCommand {
            command: command.clone(),
            detail: format!("could not start rustc: {source}"),
        })?;
        let record = CommandRecord::new(command.clone(), output);
        if record.exit_code != Some(0) {
            return Err(TuningRunnerError::VersionCommand {
                command,
                detail: command_failure("rustc version command", &record, 0),
            });
        }
        let version = String::from_utf8_lossy(&record.stdout).trim().to_owned();
        if version.is_empty() {
            return Err(TuningRunnerError::VersionCommand {
                command,
                detail: "rustc version command produced empty stdout".to_owned(),
            });
        }
        Ok(RustcVersion { record, version })
    }

    fn build_command(
        &self,
        current_dir: &Path,
        manifest_path: &Path,
        target_dir: &Path,
        rustc_args: Vec<String>,
    ) -> CommandSpec {
        let rustflags = rustc_args.join(" ");
        let mut args = vec![OsString::from("build")];
        args.extend(self.cargo_args.iter().cloned());
        args.extend([
            OsString::from("--manifest-path"),
            manifest_path.as_os_str().to_owned(),
            OsString::from("--target-dir"),
            target_dir.as_os_str().to_owned(),
            OsString::from("--release"),
        ]);
        CommandSpec {
            program: self.cargo.clone(),
            args,
            env: vec![
                (
                    OsString::from("CARGO_TARGET_DIR"),
                    Some(target_dir.as_os_str().to_owned()),
                ),
                (OsString::from("RUSTC"), Some(self.rustc.clone().into())),
                (OsString::from("RUSTC_WRAPPER"), None),
                (OsString::from("RUSTC_WORKSPACE_WRAPPER"), None),
                (OsString::from("RUSTFLAGS"), Some(OsString::from(rustflags))),
                (OsString::from("CARGO_ENCODED_RUSTFLAGS"), None),
                (OsString::from("CARGO_BUILD_RUSTFLAGS"), None),
            ],
            current_dir: current_dir.to_path_buf(),
        }
    }

    fn measure(
        &self,
        execution: &mut TuningExecution,
        current_dir: &Path,
        toolchain_version: &str,
    ) {
        if let Err(error) = self.measure_inner(execution, current_dir, toolchain_version) {
            execution.error = Some(error);
        }
    }

    fn measure_inner(
        &self,
        execution: &mut TuningExecution,
        current_dir: &Path,
        toolchain_version: &str,
    ) -> Result<(), MeasurementError> {
        fs::create_dir_all(&execution.target_dir).map_err(|error| {
            MeasurementError::new(format!(
                "could not create isolated target directory {}: {error}",
                execution.target_dir.display()
            ))
        })?;

        let output = execute(&execution.build_command).map_err(|error| {
            MeasurementError::new(format!(
                "could not start build command {}: {error}",
                execution.build_command.render()
            ))
        })?;
        let record = CommandRecord::new(execution.build_command.clone(), output);
        let stdout_path = execution.target_dir.join("build.stdout.log");
        let stderr_path = execution.target_dir.join("build.stderr.log");
        fs::write(&stdout_path, &record.stdout).map_err(|error| {
            MeasurementError::new(format!(
                "could not retain build stdout {}: {error}",
                stdout_path.display()
            ))
        })?;
        fs::write(&stderr_path, &record.stderr).map_err(|error| {
            MeasurementError::new(format!(
                "could not retain build stderr {}: {error}",
                stderr_path.display()
            ))
        })?;
        if record.exit_code != Some(0) {
            let error = command_failure("build command", &record, 0);
            execution.build_record = Some(record);
            return Err(MeasurementError::new(error));
        }
        execution.build_record = Some(record);

        let metadata = fs::metadata(&execution.artifact_path).map_err(|error| {
            MeasurementError::new(format!(
                "build did not produce expected artifact {}: {error}",
                execution.artifact_path.display()
            ))
        })?;
        if !metadata.is_file() {
            return Err(MeasurementError::new(format!(
                "expected artifact is not a regular file: {}",
                execution.artifact_path.display()
            )));
        }
        let artifact = artifact::collect_artifact(
            &execution.artifact_path,
            Some(&stdout_path),
            Some(&stderr_path),
            toolchain_version,
            &execution.build_command.render(),
        )
        .map_err(|error| {
            MeasurementError::new(format!(
                "could not collect artifact metadata for {}: {error}",
                execution.artifact_path.display()
            ))
        })?;
        execution.artifact = Some(artifact);

        let artifact_path = execution
            .artifact_path
            .to_str()
            .ok_or_else(|| MeasurementError::new("benchmark artifact path is not valid UTF-8"))?;
        let workload_program = self
            .workload_program
            .as_deref()
            .unwrap_or(&execution.artifact_path);
        let command = workload_program
            .to_str()
            .ok_or_else(|| MeasurementError::new("workload program path is not valid UTF-8"))?
            .to_owned();
        let benchmark_args = self
            .benchmark_args
            .iter()
            .map(|argument| argument.replace("{artifact}", artifact_path))
            .collect::<Vec<_>>();
        execution.benchmark_command = Some(CommandSpec {
            program: workload_program.to_path_buf(),
            args: benchmark_args.iter().map(OsString::from).collect(),
            env: Vec::new(),
            current_dir: current_dir.to_path_buf(),
        });
        let mut config = BenchmarkConfig::new(command);
        config.args = benchmark_args;
        config.runs = self.runs;
        config.warmup_runs = self.warmup_runs;
        config.expected_exit_code = self.expected_exit_code;
        config.stdin = self.benchmark_stdin.clone();
        config.stdout = self.benchmark_stdout.clone();
        let result = benchmark::run_benchmark(&config).map_err(|error| {
            MeasurementError::new(format!(
                "benchmark command {} failed: {error}",
                execution
                    .benchmark_command
                    .as_ref()
                    .expect("benchmark command was recorded")
                    .render()
            ))
        })?;
        execution.benchmark = Some(result);
        Ok(())
    }
}

pub fn run_tuning(
    runner: &TuningRunner,
    candidates: &[LlvmConfig],
) -> Result<TuningRunReport, TuningRunnerError> {
    runner.run(candidates)
}

pub fn run_default_tuning(runner: &TuningRunner) -> Result<TuningRunReport, TuningRunnerError> {
    runner.run_default()
}

fn tuning_run_report_json(report: &TuningRunReport) -> String {
    let mut json = String::from("{\"schema_version\":1,\"run_dir\":");
    push_path(&mut json, &report.run_dir);
    json.push_str(",\"configuration\":");
    push_tuning_runner(&mut json, &report.configuration);
    json.push_str(",\"rustc_version\":");
    push_rustc_version(&mut json, &report.rustc_version);
    json.push_str(",\"baseline\":");
    push_execution(&mut json, &report.baseline);
    json.push_str(",\"candidates\":[");
    for (index, candidate) in report.candidates.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        push_execution(&mut json, candidate);
    }
    write!(
        json,
        "],\"omitted_candidate_count\":{},\"selection\":",
        report.omitted_candidate_count
    )
    .unwrap();
    push_selected(&mut json, report.search.selected.as_ref());
    json.push_str(",\"search\":");
    push_search_report(&mut json, &report.search);
    json.push_str(",\"measurement_failed\":");
    json.push_str(if report.has_measurement_failures() {
        "true"
    } else {
        "false"
    });
    json.push('}');
    json
}

fn push_tuning_runner(json: &mut String, runner: &TuningRunner) {
    json.push_str("{\"manifest_path\":");
    push_path(json, &runner.manifest_path);
    json.push_str(",\"target_root\":");
    push_path(json, &runner.target_root);
    json.push_str(",\"artifact_relative_path\":");
    push_path(json, &runner.artifact_relative_path);
    json.push_str(",\"cargo\":");
    push_path(json, &runner.cargo);
    json.push_str(",\"rustc\":");
    push_path(json, &runner.rustc);
    json.push_str(",\"cargo_args\":[");
    for (index, argument) in runner.cargo_args.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        push_os_string(json, argument);
    }
    json.push_str("],\"workload_program\":");
    match &runner.workload_program {
        Some(program) => push_path(json, program),
        None => json.push_str("null"),
    }
    json.push_str(",\"benchmark_args\":[");
    for (index, argument) in runner.benchmark_args.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        push_json_string(json, argument);
    }
    json.push_str("],\"benchmark_stdin\":");
    push_redirect(json, &runner.benchmark_stdin);
    json.push_str(",\"benchmark_stdout\":");
    push_redirect(json, &runner.benchmark_stdout);
    write!(
        json,
        ",\"runs\":{},\"warmup_runs\":{},\"expected_exit_code\":{},\"max_candidates\":{}",
        runner.runs, runner.warmup_runs, runner.expected_exit_code, runner.max_candidates
    )
    .unwrap();
    json.push_str(",\"policy\":{");
    write!(json, "\"min_samples\":{}", runner.policy.min_samples).unwrap();
    json.push_str(",\"max_runtime_ratio\":");
    push_f64(json, runner.policy.max_runtime_ratio);
    json.push_str(",\"max_size_ratio\":");
    push_f64(json, runner.policy.max_size_ratio);
    json.push_str("}}");
}

fn push_redirect(json: &mut String, redirect: &Redirect) {
    match redirect {
        Redirect::Inherit => json.push_str("{\"kind\":\"inherit\"}"),
        Redirect::Null => json.push_str("{\"kind\":\"null\"}"),
        Redirect::File(path) => {
            json.push_str("{\"kind\":\"file\",\"path\":");
            push_path(json, path);
            json.push('}');
        }
    }
}

fn push_rustc_version(json: &mut String, version: &RustcVersion) {
    json.push_str("{\"record\":");
    push_command_record(json, &version.record);
    json.push_str(",\"version\":");
    push_json_string(json, &version.version);
    json.push('}');
}

fn push_execution(json: &mut String, execution: &TuningExecution) {
    json.push_str("{\"target_dir\":");
    push_path(json, &execution.target_dir);
    json.push_str(",\"artifact_path\":");
    push_path(json, &execution.artifact_path);
    json.push_str(",\"build_command\":");
    push_command_spec(json, &execution.build_command);
    json.push_str(",\"build_record\":");
    match &execution.build_record {
        Some(record) => push_command_record(json, record),
        None => json.push_str("null"),
    }
    json.push_str(",\"build_stdout_path\":");
    push_path(json, &execution.target_dir.join("build.stdout.log"));
    json.push_str(",\"build_stderr_path\":");
    push_path(json, &execution.target_dir.join("build.stderr.log"));
    json.push_str(",\"artifact\":");
    match &execution.artifact {
        Some(artifact) => json.push_str(&artifact.to_json()),
        None => json.push_str("null"),
    }
    json.push_str(",\"benchmark_command\":");
    match &execution.benchmark_command {
        Some(command) => push_command_spec(json, command),
        None => json.push_str("null"),
    }
    json.push_str(",\"benchmark\":");
    match &execution.benchmark {
        Some(benchmark) => json.push_str(&benchmark.to_json()),
        None => json.push_str("null"),
    }
    json.push_str(",\"error\":");
    match &execution.error {
        Some(error) => push_json_string(json, error.message()),
        None => json.push_str("null"),
    }
    json.push('}');
}

fn push_command_record(json: &mut String, record: &CommandRecord) {
    json.push_str("{\"command\":");
    push_command_spec(json, &record.command);
    json.push_str(",\"exit_code\":");
    match record.exit_code {
        Some(code) => write!(json, "{code}").unwrap(),
        None => json.push_str("null"),
    }
    json.push_str(",\"stdout\":");
    push_bytes(json, &record.stdout);
    json.push_str(",\"stderr\":");
    push_bytes(json, &record.stderr);
    json.push('}');
}

fn push_command_spec(json: &mut String, command: &CommandSpec) {
    json.push_str("{\"program\":");
    push_path(json, &command.program);
    json.push_str(",\"args\":[");
    for (index, argument) in command.args.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        push_os_string(json, argument);
    }
    json.push_str("],\"env\":[");
    for (index, (key, value)) in command.env.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        json.push_str("{\"key\":");
        push_os_string(json, key);
        json.push_str(",\"value\":");
        match value {
            Some(value) => push_os_string(json, value),
            None => json.push_str("null"),
        }
        json.push('}');
    }
    json.push_str("],\"current_dir\":");
    push_path(json, &command.current_dir);
    json.push_str(",\"rendered\":");
    push_json_string(json, &command.render());
    json.push('}');
}

fn push_search_report(json: &mut String, report: &SearchReport) {
    json.push_str("{\"baseline\":");
    match &report.baseline {
        Some(summary) => push_summary(json, summary),
        None => json.push_str("null"),
    }
    json.push_str(",\"baseline_rejection\":");
    push_rejection(json, report.baseline_rejection.as_ref());
    json.push_str(",\"candidates\":[");
    for (index, candidate) in report.candidates.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        push_candidate_evaluation(json, candidate);
    }
    json.push_str("],\"selected\":");
    push_selected(json, report.selected.as_ref());
    json.push('}');
}

fn push_candidate_evaluation(json: &mut String, candidate: &tuning::CandidateEvaluation) {
    json.push_str("{\"config\":");
    push_llvm_config(json, &candidate.config);
    json.push_str(",\"summary\":");
    match &candidate.summary {
        Some(summary) => push_summary(json, summary),
        None => json.push_str("null"),
    }
    json.push_str(",\"runtime_ratio\":");
    push_optional_f64(json, candidate.runtime_ratio);
    json.push_str(",\"size_ratio\":");
    push_optional_f64(json, candidate.size_ratio);
    json.push_str(",\"pareto_optimal\":");
    json.push_str(if candidate.pareto_optimal {
        "true"
    } else {
        "false"
    });
    json.push_str(",\"score\":");
    push_optional_f64(json, candidate.score);
    json.push_str(",\"rejection\":");
    push_rejection(json, candidate.rejection.as_ref());
    json.push('}');
}

fn push_selected(json: &mut String, selected: Option<&tuning::SelectedCandidate>) {
    let Some(selected) = selected else {
        json.push_str("null");
        return;
    };
    write!(json, "{{\"candidate_index\":{}", selected.candidate_index).unwrap();
    json.push_str(",\"config\":");
    push_llvm_config(json, &selected.config);
    json.push_str(",\"summary\":");
    push_summary(json, &selected.summary);
    json.push_str(",\"runtime_ratio\":");
    push_f64(json, selected.runtime_ratio);
    json.push_str(",\"size_ratio\":");
    push_f64(json, selected.size_ratio);
    json.push_str(",\"score\":");
    push_f64(json, selected.score);
    json.push('}');
}

fn push_summary(json: &mut String, summary: &tuning::MeasurementSummary) {
    json.push_str("{\"runtime_samples_seconds\":[");
    for (index, sample) in summary.runtime_samples_seconds.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        push_f64(json, *sample);
    }
    json.push_str("],\"median_seconds\":");
    push_f64(json, summary.median_seconds);
    write!(
        json,
        ",\"artifact_size_bytes\":{}}}",
        summary.artifact_size_bytes
    )
    .unwrap();
}

fn push_rejection(json: &mut String, rejection: Option<&tuning::RejectionReason>) {
    let Some(rejection) = rejection else {
        json.push_str("null");
        return;
    };
    match rejection {
        tuning::RejectionReason::MeasurementFailed(message) => {
            json.push_str("{\"kind\":\"measurement_failed\",\"message\":");
            push_json_string(json, message);
            json.push('}');
        }
        tuning::RejectionReason::InvalidConfiguration(error) => {
            json.push_str("{\"kind\":\"invalid_configuration\",\"message\":");
            push_json_string(json, &error.to_string());
            json.push('}');
        }
        tuning::RejectionReason::InsufficientSamples { required, actual } => {
            write!(
                json,
                "{{\"kind\":\"insufficient_samples\",\"required\":{required},\"actual\":{actual}}}"
            )
            .unwrap();
        }
        tuning::RejectionReason::InvalidRuntimeSample { index, value } => {
            write!(
                json,
                "{{\"kind\":\"invalid_runtime_sample\",\"index\":{index},\"value\":"
            )
            .unwrap();
            push_f64(json, *value);
            json.push('}');
        }
        tuning::RejectionReason::EmptyArtifact => {
            json.push_str("{\"kind\":\"empty_artifact\"}");
        }
        tuning::RejectionReason::RuntimeRatioExceeded { actual, maximum } => {
            json.push_str("{\"kind\":\"runtime_ratio_exceeded\",\"actual\":");
            push_f64(json, *actual);
            json.push_str(",\"maximum\":");
            push_f64(json, *maximum);
            json.push('}');
        }
        tuning::RejectionReason::SizeRatioExceeded { actual, maximum } => {
            json.push_str("{\"kind\":\"size_ratio_exceeded\",\"actual\":");
            push_f64(json, *actual);
            json.push_str(",\"maximum\":");
            push_f64(json, *maximum);
            json.push('}');
        }
    }
}

fn push_llvm_config(json: &mut String, config: &LlvmConfig) {
    let opt_level = match config.opt_level {
        crate::llvm::OptLevel::Speed => "speed",
        crate::llvm::OptLevel::Balanced => "balanced",
        crate::llvm::OptLevel::Size => "size",
    };
    let lto = match config.lto {
        crate::llvm::Lto::Thin => "thin",
        crate::llvm::Lto::Fat => "fat",
    };
    let target_cpu = match config.target_cpu {
        crate::llvm::TargetCpu::Generic => "generic",
        crate::llvm::TargetCpu::Native => "native",
    };
    json.push_str("{\"opt_level\":");
    push_json_string(json, opt_level);
    json.push_str(",\"lto\":");
    push_json_string(json, lto);
    write!(
        json,
        ",\"codegen_units\":{},\"panic_abort\":{},\"strip\":{},\"target_cpu\":",
        config.codegen_units, config.panic_abort, config.strip
    )
    .unwrap();
    push_json_string(json, target_cpu);
    json.push_str(",\"rustc_args\":");
    match config.rustc_args() {
        Ok(arguments) => {
            json.push('[');
            for (index, argument) in arguments.iter().enumerate() {
                if index != 0 {
                    json.push(',');
                }
                push_json_string(json, argument);
            }
            json.push(']');
        }
        Err(_) => json.push_str("null"),
    }
    json.push('}');
}

fn push_optional_f64(json: &mut String, value: Option<f64>) {
    match value {
        Some(value) => push_f64(json, value),
        None => json.push_str("null"),
    }
}

fn push_f64(json: &mut String, value: f64) {
    if value.is_finite() {
        write!(json, "{value}").unwrap();
    } else {
        json.push_str("null");
    }
}

fn push_path(json: &mut String, path: &Path) {
    push_os_string(json, path.as_os_str());
}

fn push_os_string(json: &mut String, value: &OsStr) {
    match value.to_str() {
        Some(value) => push_json_string(json, value),
        None => push_hex_object(json, value.as_encoded_bytes()),
    }
}

fn push_bytes(json: &mut String, value: &[u8]) {
    match std::str::from_utf8(value) {
        Ok(value) => push_json_string(json, value),
        Err(_) => push_hex_object(json, value),
    }
}

fn push_hex_object(json: &mut String, value: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    json.push_str("{\"encoding\":\"hex\",\"value\":\"");
    for byte in value {
        json.push(HEX[(byte >> 4) as usize] as char);
        json.push(HEX[(byte & 0x0f) as usize] as char);
    }
    json.push_str("\"}");
}

fn push_json_string(json: &mut String, value: &str) {
    json.push('"');
    for character in value.chars() {
        match character {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\u{08}' => json.push_str("\\b"),
            '\u{0c}' => json.push_str("\\f"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            '\u{00}'..='\u{1f}' => {
                write!(json, "\\u{:04x}", character as u32).unwrap();
            }
            _ => json.push(character),
        }
    }
    json.push('"');
}

#[derive(Debug)]
pub enum TuningRunnerError {
    InvalidConfig(String),
    Policy(PolicyError),
    VersionCommand {
        command: CommandSpec,
        detail: String,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for TuningRunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => {
                write!(formatter, "invalid tuning runner configuration: {message}")
            }
            Self::Policy(error) => write!(formatter, "invalid tuning policy: {error}"),
            Self::VersionCommand { command, detail } => write!(
                formatter,
                "could not record rustc version with {}: {detail}",
                command.render()
            ),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} at {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for TuningRunnerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Policy(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::InvalidConfig(_) | Self::VersionCommand { .. } => None,
        }
    }
}

fn validate_policy(policy: &SearchPolicy) -> Result<(), TuningRunnerError> {
    if !policy.max_runtime_ratio.is_finite() || policy.max_runtime_ratio < 0.0 {
        return Err(TuningRunnerError::Policy(PolicyError::InvalidRuntimeRatio(
            policy.max_runtime_ratio,
        )));
    }
    if !policy.max_size_ratio.is_finite() || policy.max_size_ratio < 0.0 {
        return Err(TuningRunnerError::Policy(PolicyError::InvalidSizeRatio(
            policy.max_size_ratio,
        )));
    }
    Ok(())
}

fn execute(command: &CommandSpec) -> io::Result<Output> {
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.current_dir)
        .stdin(std::process::Stdio::null());
    for (key, value) in &command.env {
        match value {
            Some(value) => process.env(key, value),
            None => process.env_remove(key),
        };
    }
    process.output()
}

fn command_failure(context: &str, record: &CommandRecord, expected: i32) -> String {
    let actual = record.exit_code.map_or_else(
        || "termination without an exit code".to_owned(),
        |code| code.to_string(),
    );
    let mut message = format!(
        "{context} exited with {actual}, expected {expected}: {}",
        record.command.render()
    );
    let stderr = String::from_utf8_lossy(&record.stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        message.push_str(&format!(": {stderr}"));
    }
    message
}

fn forbidden_fast_math<I, S>(args: I) -> Option<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    for argument in args {
        let argument = argument.as_ref();
        let lowercase = argument.to_string_lossy().to_lowercase();
        if FORBIDDEN_FAST_MATH
            .iter()
            .any(|forbidden| lowercase.contains(forbidden))
        {
            return Some(argument.to_os_string());
        }
    }
    None
}

fn absolute_path(current_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    }
}

fn create_run_directory(root: &Path) -> Result<PathBuf, TuningRunnerError> {
    fs::create_dir_all(root).map_err(|source| TuningRunnerError::Io {
        operation: "create tuning target root",
        path: root.to_path_buf(),
        source,
    })?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for _ in 0..UNIQUE_DIRECTORY_ATTEMPTS {
        let sequence = NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("run-{}-{timestamp}-{sequence}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(TuningRunnerError::Io {
                    operation: "create unique tuning run directory",
                    path,
                    source,
                });
            }
        }
    }
    Err(TuningRunnerError::Io {
        operation: "allocate a unique tuning run directory",
        path: root.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "exhausted unique directory attempts",
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llvm::OptLevel;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "kfrc-tuning-runner-test-{}-{timestamp}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn join(&self, path: &str) -> PathBuf {
            self.0.join(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn runner(directory: &TestDirectory) -> TuningRunner {
        TuningRunner::new(
            directory.join("Cargo.toml"),
            directory.join("target"),
            "release/demo",
        )
    }

    #[test]
    fn enforces_three_real_runs_and_a_candidate_limit_before_launching_tools() {
        let directory = TestDirectory::new();
        let mut runner = runner(&directory);
        runner.runs = 2;
        runner.cargo = directory.join("missing-cargo");
        runner.rustc = directory.join("missing-rustc");
        let error = runner.run(&[]).unwrap_err();
        assert!(error.to_string().contains("at least 3 runs"));
        assert!(!directory.join("target").exists());

        runner.runs = 3;
        runner.max_candidates = 0;
        let error = runner.run(&[]).unwrap_err();
        assert!(error.to_string().contains("candidate count"));

        runner.max_candidates = DEFAULT_MAX_CANDIDATES;
        runner.policy.min_samples = 2;
        let error = runner.run(&[]).unwrap_err();
        assert!(error.to_string().contains("at least 3 samples"));
    }

    #[test]
    fn rejects_fast_math_before_launching_tools() {
        let directory = TestDirectory::new();
        let mut runner = runner(&directory);
        runner.cargo_args = vec![
            OsString::from("--config"),
            OsString::from("build.rustflags=['-Cllvm-args=-enable-unsafe-fp-math']"),
        ];
        runner.cargo = directory.join("missing-cargo");
        runner.rustc = directory.join("missing-rustc");

        let error = runner.run(&[]).unwrap_err();
        assert!(error.to_string().contains("forbidden fast-math"));
        assert!(!directory.join("target").exists());
    }

    #[test]
    fn default_limit_bounds_the_sixty_configuration_search() {
        let directory = TestDirectory::new();
        let runner = runner(&directory);
        assert_eq!(tuning::default_candidate_configs().len(), 60);
        assert_eq!(runner.max_candidates, DEFAULT_MAX_CANDIDATES);
        assert!(tuning::default_candidate_configs().len() > runner.max_candidates);
        assert_eq!(runner.runs, 3);
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
    fn fake_tools(directory: &TestDirectory, fail_candidate: bool) -> TuningRunner {
        let cargo_log = directory.join("cargo.log");
        let cargo = directory.join("cargo");
        let rustc = directory.join("rustc");
        write_executable(
            &rustc,
            "#!/bin/sh\nprintf 'rustc 1.85.0 (fake)\\nrelease: 1.85.0\\n'\n",
        );
        write_executable(
            &cargo,
            &format!(
                r#"#!/bin/sh
printf 'RUSTFLAGS=%s CARGO_TARGET_DIR=%s\n' "$RUSTFLAGS" "$CARGO_TARGET_DIR" >> {log}
target=
while [ "$#" -gt 0 ]; do
    if [ "$1" = "--target-dir" ]; then
        target=$2
        shift 2
    else
        shift
    fi
done
if [ -n "$RUSTFLAGS" ] && [ "{fail_candidate}" = "true" ]; then
    printf 'injected candidate failure\n' >&2
    exit 23
fi
mkdir -p "$target/release"
if [ -n "$RUSTFLAGS" ]; then
    delay=0.01
else
    delay=0.04
fi
printf '#!/bin/sh\nsleep %s\n' "$delay" > "$target/release/demo"
if [ -z "$RUSTFLAGS" ]; then
    i=0
    while [ "$i" -lt 40 ]; do
        printf '# baseline padding padding padding padding\n' >> "$target/release/demo"
        i=$((i + 1))
    done
fi
chmod +x "$target/release/demo"
"#,
                log = shell_quote(&cargo_log),
                fail_candidate = fail_candidate,
            ),
        );
        let mut runner = runner(directory);
        runner.cargo = cargo;
        runner.rustc = rustc;
        runner.max_candidates = 1;
        runner
    }

    #[cfg(unix)]
    #[test]
    fn mock_builds_feed_real_artifact_sizes_and_three_samples_into_selection() {
        let directory = TestDirectory::new();
        let runner = fake_tools(&directory, false);
        let candidates = [
            LlvmConfig {
                opt_level: OptLevel::Size,
                ..LlvmConfig::default()
            },
            LlvmConfig::default(),
        ];

        let report = runner.run(&candidates).unwrap();

        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.omitted_candidate_count, 1);
        assert_eq!(report.rustc_version.record.exit_code, Some(0));
        assert!(report.rustc_version.version.contains("fake"));
        assert_ne!(report.baseline.target_dir, report.candidates[0].target_dir);
        assert!(report.baseline.target_dir.starts_with(&report.run_dir));
        assert!(report.candidates[0].target_dir.starts_with(&report.run_dir));
        assert_eq!(report.baseline.benchmark.as_ref().unwrap().samples.len(), 3);
        assert_eq!(
            report.candidates[0]
                .benchmark
                .as_ref()
                .unwrap()
                .samples
                .len(),
            3
        );
        let baseline_size = report.baseline.artifact.as_ref().unwrap().size_bytes;
        let candidate_size = report.candidates[0].artifact.as_ref().unwrap().size_bytes;
        assert!(candidate_size < baseline_size);
        assert_eq!(report.search.selected.as_ref().unwrap().candidate_index, 0);
        assert_eq!(
            report.search.candidates[0]
                .summary
                .as_ref()
                .unwrap()
                .artifact_size_bytes,
            candidate_size
        );
        assert!(
            report
                .baseline
                .build_command
                .render()
                .contains("Cargo.toml")
        );
        assert!(
            report.candidates[0]
                .build_command
                .render()
                .contains("opt-level=z")
        );
        assert_eq!(
            fs::read_to_string(directory.join("cargo.log"))
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[cfg(unix)]
    #[test]
    fn json_report_preserves_selection_configuration_and_complete_execution_evidence() {
        let directory = TestDirectory::new();
        let runner = fake_tools(&directory, false);

        let report = runner.run(&[LlvmConfig::default()]).unwrap();
        let json = report.to_json();

        assert!(!report.has_measurement_failures());
        assert!(json.starts_with("{\"schema_version\":1,\"run_dir\":"));
        assert!(json.contains("\"configuration\":{\"manifest_path\":"));
        assert!(json.contains("\"max_candidates\":1"));
        assert!(json.contains("\"selection\":{\"candidate_index\":0"));
        assert!(json.contains("\"build_record\":{\"command\":"));
        assert!(json.contains("build.stdout.log"));
        assert!(json.contains("\"benchmark_command\":{\"program\":"));
        assert!(json.contains("\"samples\":["));
        assert!(json.contains("\"toolchain_fingerprint\":"));
        assert!(json.ends_with("\"measurement_failed\":false}"));

        let mut encoded = String::new();
        push_bytes(&mut encoded, &[0xff, 0x00, b'a']);
        assert_eq!(encoded, "{\"encoding\":\"hex\",\"value\":\"ff0061\"}");
        let mut escaped = String::new();
        push_json_string(&mut escaped, "a\"b\\c\n\u{1f}");
        assert_eq!(escaped, "\"a\\\"b\\\\c\\n\\u001f\"");
    }

    #[cfg(unix)]
    #[test]
    fn custom_workload_receives_each_candidate_artifact_path() {
        let directory = TestDirectory::new();
        let mut runner = fake_tools(&directory, false);
        runner.workload_program = Some(PathBuf::from("/bin/sh"));
        runner.benchmark_args = vec![
            "-c".to_owned(),
            "test -x \"$1\"".to_owned(),
            "sh".to_owned(),
            "{artifact}".to_owned(),
        ];

        let report = runner.run(&[LlvmConfig::default()]).unwrap();

        assert!(!report.has_measurement_failures());
        for execution in std::iter::once(&report.baseline).chain(&report.candidates) {
            let command = execution.benchmark_command.as_ref().unwrap();
            assert_eq!(command.program, PathBuf::from("/bin/sh"));
            assert_eq!(
                command.args.last(),
                Some(&execution.artifact_path.as_os_str().to_owned())
            );
            let benchmark = execution.benchmark.as_ref().unwrap();
            assert_eq!(benchmark.command, "/bin/sh");
            assert_eq!(
                benchmark.args.last().map(String::as_str),
                execution.artifact_path.to_str()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_candidate_build_has_no_samples_and_cannot_be_selected() {
        let directory = TestDirectory::new();
        let runner = fake_tools(&directory, true);

        let report = runner.run(&[LlvmConfig::default()]).unwrap();

        assert!(report.search.selected.is_none());
        assert!(report.has_measurement_failures());
        assert!(report.to_json().ends_with("\"measurement_failed\":true}"));
        let candidate = &report.candidates[0];
        assert_eq!(candidate.build_record.as_ref().unwrap().exit_code, Some(23));
        assert!(candidate.benchmark.is_none());
        assert!(candidate.artifact.is_none());
        assert!(candidate.error.as_ref().unwrap().message().contains("23"));
        assert!(report.search.candidates[0].summary.is_none());
    }

    #[test]
    fn real_tiny_crate_builds_and_measures_when_cargo_is_available() {
        let directory = TestDirectory::new();
        let mut runner = TuningRunner::new(
            directory.join("Cargo.toml"),
            directory.join("target"),
            "release/tiny-tuning",
        );
        if Command::new(&runner.cargo)
            .arg("--version")
            .output()
            .is_err()
            || Command::new(&runner.rustc)
                .arg("--version")
                .output()
                .is_err()
        {
            return;
        }

        fs::create_dir(directory.join("src")).unwrap();
        fs::write(
            directory.join("Cargo.toml"),
            "[package]\nname = \"tiny-tuning\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            directory.join("src/main.rs"),
            "fn main() {\n    let mut value = 0_u64;\n    for i in 0..10_000 { value = value.wrapping_mul(33).wrapping_add(i); }\n    println!(\"{value}\");\n}\n",
        )
        .unwrap();
        runner.cargo_args = vec![OsString::from("--offline")];
        runner.max_candidates = 1;

        let report = runner.run(&[LlvmConfig::default()]).unwrap();

        assert!(report.rustc_version.version.contains("rustc"));
        assert!(
            report.baseline.error.is_none(),
            "{:?}",
            report.baseline.error
        );
        assert!(
            report.candidates[0].error.is_none(),
            "{:?}",
            report.candidates[0].error
        );
        assert_eq!(
            report
                .baseline
                .measurement()
                .unwrap()
                .runtime_samples_seconds
                .len(),
            3
        );
        assert_eq!(
            report.candidates[0]
                .measurement()
                .unwrap()
                .runtime_samples_seconds
                .len(),
            3
        );
        assert!(report.baseline.artifact.as_ref().unwrap().size_bytes > 0);
        assert!(report.candidates[0].artifact.as_ref().unwrap().size_bytes > 0);
        if let Some(selected) = &report.search.selected {
            assert!(
                report.candidates[selected.candidate_index]
                    .measurement()
                    .is_ok()
            );
        }
    }
}
