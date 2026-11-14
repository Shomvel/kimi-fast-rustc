//! Deterministic optimization search for the LLVM reference track.
//!
//! Selection uses only the supplied measurements. It does not predict
//! unmeasured improvements, and native-CPU results remain specific to the host
//! on which they were measured.

use crate::llvm::{ConfigError, LlvmConfig, Lto, OptLevel, TargetCpu};
use std::error::Error;
use std::fmt;

pub const DEFAULT_MIN_SAMPLES: usize = 3;
pub const DEFAULT_MAX_RUNTIME_RATIO: f64 = 1.0;
pub const DEFAULT_MAX_SIZE_RATIO: f64 = 1.0;
pub const DEFAULT_CODEGEN_UNITS: [usize; 5] = [1, 2, 4, 8, 16];

#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    pub runtime_samples_seconds: Vec<f64>,
    pub artifact_size_bytes: u64,
}

impl Measurement {
    pub fn new(runtime_samples_seconds: Vec<f64>, artifact_size_bytes: u64) -> Self {
        Self {
            runtime_samples_seconds,
            artifact_size_bytes,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeasurementError {
    message: String,
}

impl MeasurementError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MeasurementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for MeasurementError {}

impl From<String> for MeasurementError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for MeasurementError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateMeasurement {
    pub config: LlvmConfig,
    pub measurement: Result<Measurement, MeasurementError>,
}

impl CandidateMeasurement {
    pub fn new(config: LlvmConfig, measurement: Measurement) -> Self {
        Self {
            config,
            measurement: Ok(measurement),
        }
    }

    pub fn failed(config: LlvmConfig, error: impl Into<MeasurementError>) -> Self {
        Self {
            config,
            measurement: Err(error.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchPolicy {
    pub min_samples: usize,
    pub max_runtime_ratio: f64,
    pub max_size_ratio: f64,
}

impl Default for SearchPolicy {
    fn default() -> Self {
        Self {
            min_samples: DEFAULT_MIN_SAMPLES,
            max_runtime_ratio: DEFAULT_MAX_RUNTIME_RATIO,
            max_size_ratio: DEFAULT_MAX_SIZE_RATIO,
        }
    }
}

impl SearchPolicy {
    fn validate(&self) -> Result<(), PolicyError> {
        if self.min_samples == 0 {
            return Err(PolicyError::ZeroMinimumSamples);
        }
        if !self.max_runtime_ratio.is_finite() || self.max_runtime_ratio < 0.0 {
            return Err(PolicyError::InvalidRuntimeRatio(self.max_runtime_ratio));
        }
        if !self.max_size_ratio.is_finite() || self.max_size_ratio < 0.0 {
            return Err(PolicyError::InvalidSizeRatio(self.max_size_ratio));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PolicyError {
    ZeroMinimumSamples,
    InvalidRuntimeRatio(f64),
    InvalidSizeRatio(f64),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroMinimumSamples => {
                formatter.write_str("minimum runtime sample count must be greater than zero")
            }
            Self::InvalidRuntimeRatio(value) => {
                write!(
                    formatter,
                    "maximum runtime ratio must be finite and nonnegative (got {value})"
                )
            }
            Self::InvalidSizeRatio(value) => {
                write!(
                    formatter,
                    "maximum size ratio must be finite and nonnegative (got {value})"
                )
            }
        }
    }
}

impl Error for PolicyError {}

#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementSummary {
    pub runtime_samples_seconds: Vec<f64>,
    pub median_seconds: f64,
    pub artifact_size_bytes: u64,
}

impl MeasurementSummary {
    fn new(measurement: &Measurement, min_samples: usize) -> Result<Self, RejectionReason> {
        let sample_count = measurement.runtime_samples_seconds.len();
        if sample_count < min_samples {
            return Err(RejectionReason::InsufficientSamples {
                required: min_samples,
                actual: sample_count,
            });
        }
        for (index, value) in measurement
            .runtime_samples_seconds
            .iter()
            .copied()
            .enumerate()
        {
            if !value.is_finite() || value <= 0.0 {
                return Err(RejectionReason::InvalidRuntimeSample { index, value });
            }
        }
        if measurement.artifact_size_bytes == 0 {
            return Err(RejectionReason::EmptyArtifact);
        }

        let mut sorted = measurement.runtime_samples_seconds.clone();
        sorted.sort_by(f64::total_cmp);
        let middle = sorted.len() / 2;
        let median_seconds = if sorted.len() % 2 == 0 {
            sorted[middle - 1] + (sorted[middle] - sorted[middle - 1]) / 2.0
        } else {
            sorted[middle]
        };

        Ok(Self {
            runtime_samples_seconds: measurement.runtime_samples_seconds.clone(),
            median_seconds,
            artifact_size_bytes: measurement.artifact_size_bytes,
        })
    }

    pub fn sample_count(&self) -> usize {
        self.runtime_samples_seconds.len()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RejectionReason {
    MeasurementFailed(String),
    InvalidConfiguration(ConfigError),
    InsufficientSamples { required: usize, actual: usize },
    InvalidRuntimeSample { index: usize, value: f64 },
    EmptyArtifact,
    RuntimeRatioExceeded { actual: f64, maximum: f64 },
    SizeRatioExceeded { actual: f64, maximum: f64 },
}

impl fmt::Display for RejectionReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MeasurementFailed(message) => write!(formatter, "measurement failed: {message}"),
            Self::InvalidConfiguration(error) => {
                write!(formatter, "invalid LLVM configuration: {error}")
            }
            Self::InsufficientSamples { required, actual } => write!(
                formatter,
                "insufficient runtime samples: required {required}, got {actual}"
            ),
            Self::InvalidRuntimeSample { index, value } => write!(
                formatter,
                "runtime sample {index} must be finite and greater than zero (got {value})"
            ),
            Self::EmptyArtifact => formatter.write_str("artifact size must be greater than zero"),
            Self::RuntimeRatioExceeded { actual, maximum } => write!(
                formatter,
                "median runtime ratio {actual} exceeds maximum {maximum}"
            ),
            Self::SizeRatioExceeded { actual, maximum } => write!(
                formatter,
                "artifact size ratio {actual} exceeds maximum {maximum}"
            ),
        }
    }
}

impl Error for RejectionReason {}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateEvaluation {
    pub config: LlvmConfig,
    pub summary: Option<MeasurementSummary>,
    pub runtime_ratio: Option<f64>,
    pub size_ratio: Option<f64>,
    pub pareto_optimal: bool,
    pub score: Option<f64>,
    pub rejection: Option<RejectionReason>,
}

impl CandidateEvaluation {
    fn new(config: LlvmConfig) -> Self {
        Self {
            config,
            summary: None,
            runtime_ratio: None,
            size_ratio: None,
            pareto_optimal: false,
            score: None,
            rejection: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectedCandidate {
    pub candidate_index: usize,
    pub config: LlvmConfig,
    pub summary: MeasurementSummary,
    pub runtime_ratio: f64,
    pub size_ratio: f64,
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchReport {
    pub baseline: Option<MeasurementSummary>,
    pub baseline_rejection: Option<RejectionReason>,
    pub candidates: Vec<CandidateEvaluation>,
    pub selected: Option<SelectedCandidate>,
}

pub fn default_candidate_configs() -> Vec<LlvmConfig> {
    candidate_configs(&DEFAULT_CODEGEN_UNITS)
        .expect("default tuning codegen units must be greater than zero")
}

pub fn candidate_configs(codegen_units: &[usize]) -> Result<Vec<LlvmConfig>, ConfigError> {
    let mut codegen_units = codegen_units.to_vec();
    codegen_units.sort_unstable();
    codegen_units.dedup();

    let opt_levels = [OptLevel::Speed, OptLevel::Balanced, OptLevel::Size];
    let lto_modes = [Lto::Thin, Lto::Fat];
    let target_cpus = [TargetCpu::Generic, TargetCpu::Native];
    let mut configs = Vec::with_capacity(
        opt_levels.len() * lto_modes.len() * codegen_units.len() * target_cpus.len(),
    );

    for opt_level in opt_levels {
        for lto in lto_modes {
            for &codegen_units in &codegen_units {
                for target_cpu in target_cpus {
                    let config = LlvmConfig {
                        opt_level,
                        lto,
                        codegen_units,
                        target_cpu,
                        ..LlvmConfig::default()
                    };
                    config.validate()?;
                    configs.push(config);
                }
            }
        }
    }
    Ok(configs)
}

pub fn search(
    baseline: Result<Measurement, MeasurementError>,
    candidates: &[CandidateMeasurement],
    policy: &SearchPolicy,
) -> Result<SearchReport, PolicyError> {
    policy.validate()?;

    let (baseline_summary, baseline_rejection) = match baseline {
        Ok(measurement) => match MeasurementSummary::new(&measurement, policy.min_samples) {
            Ok(summary) => (Some(summary), None),
            Err(reason) => (None, Some(reason)),
        },
        Err(error) => (
            None,
            Some(RejectionReason::MeasurementFailed(error.to_string())),
        ),
    };

    let mut evaluations: Vec<_> = candidates
        .iter()
        .map(|candidate| {
            let mut evaluation = CandidateEvaluation::new(candidate.config);
            if let Err(error) = candidate.config.validate() {
                evaluation.rejection = Some(RejectionReason::InvalidConfiguration(error));
                return evaluation;
            }
            match &candidate.measurement {
                Ok(measurement) => match MeasurementSummary::new(measurement, policy.min_samples) {
                    Ok(summary) => evaluation.summary = Some(summary),
                    Err(reason) => evaluation.rejection = Some(reason),
                },
                Err(error) => {
                    evaluation.rejection =
                        Some(RejectionReason::MeasurementFailed(error.to_string()));
                }
            }
            evaluation
        })
        .collect();

    let mut metrics = vec![None; evaluations.len()];
    if let Some(baseline) = &baseline_summary {
        for (index, evaluation) in evaluations.iter_mut().enumerate() {
            let Some(summary) = &evaluation.summary else {
                continue;
            };
            let runtime_ratio = summary.median_seconds / baseline.median_seconds;
            let size_ratio =
                summary.artifact_size_bytes as f64 / baseline.artifact_size_bytes as f64;
            evaluation.runtime_ratio = Some(runtime_ratio);
            evaluation.size_ratio = Some(size_ratio);

            if !within_runtime_threshold(
                summary.median_seconds,
                baseline.median_seconds,
                policy.max_runtime_ratio,
            ) {
                evaluation.rejection = Some(RejectionReason::RuntimeRatioExceeded {
                    actual: runtime_ratio,
                    maximum: policy.max_runtime_ratio,
                });
                continue;
            }
            if !within_size_threshold(
                summary.artifact_size_bytes,
                baseline.artifact_size_bytes,
                policy.max_size_ratio,
            ) {
                evaluation.rejection = Some(RejectionReason::SizeRatioExceeded {
                    actual: size_ratio,
                    maximum: policy.max_size_ratio,
                });
                continue;
            }

            let score = runtime_ratio * 0.5 + size_ratio * 0.5;
            evaluation.score = Some(score);
            metrics[index] = Some((runtime_ratio, size_ratio, score));
        }

        for index in 0..evaluations.len() {
            let Some((runtime_ratio, size_ratio, _)) = metrics[index] else {
                continue;
            };
            let dominated = metrics.iter().enumerate().any(|(other, metric)| {
                let Some((other_runtime, other_size, _)) = *metric else {
                    return false;
                };
                other != index
                    && other_runtime <= runtime_ratio
                    && other_size <= size_ratio
                    && (other_runtime < runtime_ratio || other_size < size_ratio)
            });
            evaluations[index].pareto_optimal = !dominated;
        }
    }

    let selected_index = evaluations
        .iter()
        .enumerate()
        .filter(|(_, evaluation)| evaluation.pareto_optimal)
        .map(|(index, _)| index)
        .min_by(|left, right| compare_candidates(*left, *right, &evaluations, &metrics));

    let selected = selected_index.map(|candidate_index| {
        let evaluation = &evaluations[candidate_index];
        let (runtime_ratio, size_ratio, score) = metrics[candidate_index]
            .expect("a Pareto-optimal candidate must have finite comparison metrics");
        SelectedCandidate {
            candidate_index,
            config: evaluation.config,
            summary: evaluation
                .summary
                .clone()
                .expect("a selected candidate must have a measurement summary"),
            runtime_ratio,
            size_ratio,
            score,
        }
    });

    Ok(SearchReport {
        baseline: baseline_summary,
        baseline_rejection,
        candidates: evaluations,
        selected,
    })
}

fn within_runtime_threshold(candidate: f64, baseline: f64, maximum: f64) -> bool {
    if maximum == 1.0 {
        candidate <= baseline
    } else {
        candidate <= baseline * maximum
    }
}

fn within_size_threshold(candidate: u64, baseline: u64, maximum: f64) -> bool {
    if maximum == 1.0 {
        candidate <= baseline
    } else {
        candidate as f64 <= baseline as f64 * maximum
    }
}

fn compare_candidates(
    left: usize,
    right: usize,
    evaluations: &[CandidateEvaluation],
    metrics: &[Option<(f64, f64, f64)>],
) -> std::cmp::Ordering {
    let (left_runtime, left_size, left_score) =
        metrics[left].expect("only measured candidates participate in winner comparison");
    let (right_runtime, right_size, right_score) =
        metrics[right].expect("only measured candidates participate in winner comparison");

    left_score
        .total_cmp(&right_score)
        .then_with(|| left_runtime.total_cmp(&right_runtime))
        .then_with(|| left_size.total_cmp(&right_size))
        .then_with(|| {
            config_order_key(&evaluations[left].config)
                .cmp(&config_order_key(&evaluations[right].config))
        })
        .then_with(|| left.cmp(&right))
}

fn config_order_key(config: &LlvmConfig) -> (u8, u8, usize, u8, bool, bool) {
    let opt_level = match config.opt_level {
        OptLevel::Speed => 0,
        OptLevel::Balanced => 1,
        OptLevel::Size => 2,
    };
    let lto = match config.lto {
        Lto::Thin => 0,
        Lto::Fat => 1,
    };
    let target_cpu = match config.target_cpu {
        TargetCpu::Generic => 0,
        TargetCpu::Native => 1,
    };
    (
        opt_level,
        lto,
        config.codegen_units,
        target_cpu,
        config.panic_abort,
        config.strip,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(opt_level: OptLevel) -> LlvmConfig {
        LlvmConfig {
            opt_level,
            ..LlvmConfig::default()
        }
    }

    fn measurement(samples: &[f64], size: u64) -> Measurement {
        Measurement::new(samples.to_vec(), size)
    }

    fn baseline(samples: &[f64], size: u64) -> Result<Measurement, MeasurementError> {
        Ok(measurement(samples, size))
    }

    fn measured(opt_level: OptLevel, samples: &[f64], size: u64) -> CandidateMeasurement {
        CandidateMeasurement::new(config(opt_level), measurement(samples, size))
    }

    #[test]
    fn candidate_generation_is_complete_sorted_and_reproducible() {
        let first = candidate_configs(&[2, 1, 2]).unwrap();
        let second = candidate_configs(&[1, 2]).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.len(), 24);
        assert_eq!(default_candidate_configs().len(), 60);
        assert_eq!(first[0].opt_level, OptLevel::Speed);
        assert_eq!(first[0].lto, Lto::Thin);
        assert_eq!(first[0].codegen_units, 1);
        assert_eq!(first[0].target_cpu, TargetCpu::Generic);
        assert_eq!(first[1].target_cpu, TargetCpu::Native);
        assert_eq!(first[2].codegen_units, 2);
        assert_eq!(first[4].lto, Lto::Fat);
        assert!(
            first
                .iter()
                .any(|config| config.opt_level == OptLevel::Balanced)
        );
        assert!(
            first
                .iter()
                .any(|config| config.opt_level == OptLevel::Size)
        );
    }

    #[test]
    fn zero_codegen_units_are_rejected_during_generation_and_search() {
        assert_eq!(
            candidate_configs(&[1, 0]),
            Err(ConfigError::InvalidCodegenUnits(0))
        );

        let invalid = LlvmConfig {
            codegen_units: 0,
            ..LlvmConfig::default()
        };
        let candidates = vec![CandidateMeasurement::new(
            invalid,
            measurement(&[1.0, 1.0, 1.0], 100),
        )];
        let report = search(
            baseline(&[10.0, 10.0, 10.0], 1_000),
            &candidates,
            &SearchPolicy::default(),
        )
        .unwrap();
        assert!(report.selected.is_none());
        assert_eq!(
            report.candidates[0].rejection,
            Some(RejectionReason::InvalidConfiguration(
                ConfigError::InvalidCodegenUnits(0)
            ))
        );
    }

    #[test]
    fn generated_configurations_do_not_enable_unsafe_fast_math() {
        let forbidden = ["fast-math", "unsafe-fp", "reassoc", "nnan", "ninf"];

        for config in default_candidate_configs() {
            let args = config.rustc_args().unwrap();
            assert!(
                args.iter()
                    .all(|arg| forbidden.iter().all(|flag| !arg.contains(flag)))
            );
        }
    }

    #[test]
    fn median_uses_sorted_samples_without_changing_evidence() {
        let summary = MeasurementSummary::new(&measurement(&[4.0, 1.0, 3.0, 2.0], 10), 4).unwrap();

        assert_eq!(summary.median_seconds, 2.5);
        assert_eq!(summary.runtime_samples_seconds, [4.0, 1.0, 3.0, 2.0]);
        assert_eq!(summary.sample_count(), 4);

        let large = MeasurementSummary::new(&measurement(&[f64::MAX, f64::MAX], 1), 2).unwrap();
        assert_eq!(large.median_seconds, f64::MAX);
    }

    #[test]
    fn search_uses_pareto_tradeoffs_between_runtime_and_size() {
        let candidates = vec![
            measured(OptLevel::Speed, &[8.5, 8.5, 8.5], 1_000),
            measured(OptLevel::Balanced, &[7.0, 7.0, 7.0], 1_100),
            measured(OptLevel::Size, &[11.0, 11.0, 11.0], 800),
            CandidateMeasurement::new(
                LlvmConfig {
                    opt_level: OptLevel::Size,
                    codegen_units: 2,
                    ..LlvmConfig::default()
                },
                measurement(&[9.0, 9.0, 9.0], 900),
            ),
        ];

        let report = search(
            baseline(&[10.0, 10.0, 10.0], 1_000),
            &candidates,
            &SearchPolicy::default(),
        )
        .unwrap();

        assert!(report.candidates[0].pareto_optimal);
        assert!(report.candidates[3].pareto_optimal);
        assert!(matches!(
            report.candidates[1].rejection,
            Some(RejectionReason::SizeRatioExceeded { .. })
        ));
        assert!(matches!(
            report.candidates[2].rejection,
            Some(RejectionReason::RuntimeRatioExceeded { .. })
        ));
        let selected = report.selected.unwrap();
        assert_eq!(selected.candidate_index, 3);
        assert_eq!(selected.runtime_ratio, 0.9);
        assert_eq!(selected.size_ratio, 0.9);
        assert_eq!(selected.score, 0.9);
    }

    #[test]
    fn dominated_candidate_is_not_on_the_pareto_frontier() {
        let candidates = vec![
            measured(OptLevel::Speed, &[9.0, 9.0, 9.0], 900),
            measured(OptLevel::Balanced, &[9.5, 9.5, 9.5], 950),
        ];

        let report = search(
            baseline(&[10.0, 10.0, 10.0], 1_000),
            &candidates,
            &SearchPolicy::default(),
        )
        .unwrap();

        assert!(report.candidates[0].pareto_optimal);
        assert!(!report.candidates[1].pareto_optimal);
        assert_eq!(report.selected.unwrap().candidate_index, 0);
    }

    #[test]
    fn default_constraints_include_the_exact_baseline_boundary() {
        let candidates = vec![measured(OptLevel::Speed, &[10.0, 10.0, 10.0], 1_000)];
        let report = search(
            baseline(&[10.0, 10.0, 10.0], 1_000),
            &candidates,
            &SearchPolicy::default(),
        )
        .unwrap();

        assert!(report.selected.is_some());
        assert_eq!(report.candidates[0].runtime_ratio, Some(1.0));
        assert_eq!(report.candidates[0].size_ratio, Some(1.0));
    }

    #[test]
    fn insufficient_samples_never_produce_a_winner() {
        let candidates = vec![measured(OptLevel::Speed, &[1.0, 1.0], 100)];
        let report = search(
            baseline(&[10.0, 10.0, 10.0], 1_000),
            &candidates,
            &SearchPolicy::default(),
        )
        .unwrap();

        assert!(report.selected.is_none());
        assert_eq!(
            report.candidates[0].rejection,
            Some(RejectionReason::InsufficientSamples {
                required: 3,
                actual: 2,
            })
        );

        let report = search(
            baseline(&[10.0, 10.0], 1_000),
            &[],
            &SearchPolicy::default(),
        )
        .unwrap();
        assert!(report.selected.is_none());
        assert_eq!(
            report.baseline_rejection,
            Some(RejectionReason::InsufficientSamples {
                required: 3,
                actual: 2,
            })
        );
    }

    #[test]
    fn failed_or_invalid_measurements_are_not_selectable() {
        let candidates = vec![
            CandidateMeasurement::failed(config(OptLevel::Speed), "compiler exited with 1"),
            measured(OptLevel::Balanced, &[1.0, f64::NAN, 1.0], 100),
            measured(OptLevel::Size, &[1.0, 1.0, 1.0], 0),
        ];
        let report = search(
            baseline(&[10.0, 10.0, 10.0], 1_000),
            &candidates,
            &SearchPolicy::default(),
        )
        .unwrap();

        assert!(report.selected.is_none());
        assert_eq!(
            report.candidates[0].rejection,
            Some(RejectionReason::MeasurementFailed(
                "compiler exited with 1".into()
            ))
        );
        assert!(matches!(
            report.candidates[1].rejection,
            Some(RejectionReason::InvalidRuntimeSample { index: 1, .. })
        ));
        assert_eq!(
            report.candidates[2].rejection,
            Some(RejectionReason::EmptyArtifact)
        );
    }

    #[test]
    fn one_failed_candidate_does_not_hide_a_valid_measurement() {
        let candidates = vec![
            CandidateMeasurement::failed(config(OptLevel::Speed), "benchmark timed out"),
            measured(OptLevel::Balanced, &[9.0, 9.0, 9.0], 900),
        ];
        let report = search(
            baseline(&[10.0, 10.0, 10.0], 1_000),
            &candidates,
            &SearchPolicy::default(),
        )
        .unwrap();

        assert_eq!(report.selected.unwrap().candidate_index, 1);
    }

    #[test]
    fn failed_baseline_disables_selection_even_with_valid_candidates() {
        let candidates = vec![measured(OptLevel::Speed, &[1.0, 1.0, 1.0], 100)];
        let report = search(
            Err(MeasurementError::new("baseline build failed")),
            &candidates,
            &SearchPolicy::default(),
        )
        .unwrap();

        assert!(report.selected.is_none());
        assert_eq!(
            report.baseline_rejection,
            Some(RejectionReason::MeasurementFailed(
                "baseline build failed".into()
            ))
        );
        assert!(report.candidates[0].summary.is_some());
        assert!(report.candidates[0].runtime_ratio.is_none());
    }

    #[test]
    fn ties_have_a_stable_configuration_order_independent_of_input_order() {
        let forward = vec![
            measured(OptLevel::Balanced, &[9.0, 9.0, 9.0], 900),
            measured(OptLevel::Speed, &[9.0, 9.0, 9.0], 900),
        ];
        let reverse = vec![forward[1].clone(), forward[0].clone()];

        let first = search(
            baseline(&[10.0, 10.0, 10.0], 1_000),
            &forward,
            &SearchPolicy::default(),
        )
        .unwrap()
        .selected
        .unwrap();
        let second = search(
            baseline(&[10.0, 10.0, 10.0], 1_000),
            &reverse,
            &SearchPolicy::default(),
        )
        .unwrap()
        .selected
        .unwrap();

        assert_eq!(first.config, config(OptLevel::Speed));
        assert_eq!(second.config, first.config);
    }

    #[test]
    fn custom_thresholds_and_sample_count_are_enforced() {
        let candidates = vec![measured(OptLevel::Speed, &[10.4, 10.4], 1_050)];
        let policy = SearchPolicy {
            min_samples: 2,
            max_runtime_ratio: 1.05,
            max_size_ratio: 1.05,
        };
        let report = search(baseline(&[10.0, 10.0], 1_000), &candidates, &policy).unwrap();
        assert!(report.selected.is_some());

        let stricter = SearchPolicy {
            max_runtime_ratio: 1.03,
            ..policy
        };
        let report = search(baseline(&[10.0, 10.0], 1_000), &candidates, &stricter).unwrap();
        assert!(report.selected.is_none());
        assert!(matches!(
            report.candidates[0].rejection,
            Some(RejectionReason::RuntimeRatioExceeded { .. })
        ));
    }

    #[test]
    fn invalid_policies_are_rejected_before_selection() {
        for policy in [
            SearchPolicy {
                min_samples: 0,
                ..SearchPolicy::default()
            },
            SearchPolicy {
                max_runtime_ratio: f64::NAN,
                ..SearchPolicy::default()
            },
            SearchPolicy {
                max_size_ratio: f64::INFINITY,
                ..SearchPolicy::default()
            },
            SearchPolicy {
                max_size_ratio: -0.1,
                ..SearchPolicy::default()
            },
        ] {
            assert!(search(baseline(&[1.0, 1.0, 1.0], 1), &[], &policy).is_err());
        }
    }
}
