use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirect {
    Inherit,
    Null,
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkConfig {
    pub command: String,
    pub args: Vec<String>,
    pub runs: usize,
    pub warmup_runs: usize,
    pub expected_exit_code: i32,
    pub stdin: Redirect,
    pub stdout: Redirect,
}

impl BenchmarkConfig {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            runs: 10,
            warmup_runs: 0,
            expected_exit_code: 0,
            stdin: Redirect::Inherit,
            stdout: Redirect::Inherit,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkResult {
    pub command: String,
    pub args: Vec<String>,
    pub warmup_runs: usize,
    pub expected_exit_code: i32,
    pub samples: Vec<f64>,
    pub mean: f64,
    pub median: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
}

impl BenchmarkResult {
    pub fn to_json(&self) -> String {
        let mut json = String::from("{");
        write!(json, "\"command\":{}", json_string(&self.command)).unwrap();
        json.push_str(",\"args\":[");
        for (index, arg) in self.args.iter().enumerate() {
            if index != 0 {
                json.push(',');
            }
            json.push_str(&json_string(arg));
        }
        write!(
            json,
            "],\"runs\":{},\"warmup_runs\":{},\"expected_exit_code\":{}",
            self.samples.len(),
            self.warmup_runs,
            self.expected_exit_code
        )
        .unwrap();
        write!(
            json,
            ",\"unit\":\"seconds\",\"mean\":{},\"median\":{},\"stddev\":{},\"min\":{},\"max\":{}",
            json_number(self.mean),
            json_number(self.median),
            json_number(self.std_dev),
            json_number(self.min),
            json_number(self.max)
        )
        .unwrap();
        json.push_str(",\"samples\":[");
        for (index, sample) in self.samples.iter().enumerate() {
            if index != 0 {
                json.push(',');
            }
            json.push_str(&json_number(*sample));
        }
        json.push_str("]}");
        json
    }
}

#[derive(Debug)]
pub enum BenchmarkError {
    ZeroRuns,
    Io(io::Error),
    UnexpectedExitCode {
        expected: i32,
        actual: Option<i32>,
        iteration: usize,
        warmup: bool,
    },
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroRuns => write!(formatter, "benchmark requires at least one timed run"),
            Self::Io(error) => write!(formatter, "benchmark I/O error: {error}"),
            Self::UnexpectedExitCode {
                expected,
                actual,
                iteration,
                warmup,
            } => {
                let phase = if *warmup { "warmup" } else { "timed" };
                match actual {
                    Some(actual) => write!(
                        formatter,
                        "{phase} run {iteration} exited with code {actual}, expected {expected}"
                    ),
                    None => write!(
                        formatter,
                        "{phase} run {iteration} terminated without an exit code, expected {expected}"
                    ),
                }
            }
        }
    }
}

impl Error for BenchmarkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for BenchmarkError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn run_benchmark(config: &BenchmarkConfig) -> Result<BenchmarkResult, BenchmarkError> {
    if config.runs == 0 {
        return Err(BenchmarkError::ZeroRuns);
    }

    for iteration in 0..config.warmup_runs {
        run_once(config, iteration, true)?;
    }

    let mut samples = Vec::with_capacity(config.runs);
    for iteration in 0..config.runs {
        samples.push(run_once(config, iteration, false)?);
    }

    let statistics = summarize(&samples);

    Ok(BenchmarkResult {
        command: config.command.clone(),
        args: config.args.clone(),
        warmup_runs: config.warmup_runs,
        expected_exit_code: config.expected_exit_code,
        samples,
        mean: statistics.mean,
        median: statistics.median,
        std_dev: statistics.std_dev,
        min: statistics.min,
        max: statistics.max,
    })
}

struct Statistics {
    mean: f64,
    median: f64,
    std_dev: f64,
    min: f64,
    max: f64,
}

fn summarize(samples: &[f64]) -> Statistics {
    assert!(!samples.is_empty());
    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let middle = sorted.len() / 2;
    let median = if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    };
    let std_dev = if samples.len() == 1 {
        0.0
    } else {
        (samples
            .iter()
            .map(|sample| {
                let difference = sample - mean;
                difference * difference
            })
            .sum::<f64>()
            / (samples.len() - 1) as f64)
            .sqrt()
    };

    Statistics {
        mean,
        median,
        std_dev,
        min: sorted[0],
        max: sorted[sorted.len() - 1],
    }
}

fn run_once(
    config: &BenchmarkConfig,
    iteration: usize,
    warmup: bool,
) -> Result<f64, BenchmarkError> {
    let mut command = Command::new(&config.command);
    command
        .args(&config.args)
        .stdin(redirect_stdio(&config.stdin, true)?)
        .stdout(redirect_stdio(&config.stdout, false)?);

    let started = Instant::now();
    let status = command.status()?;
    let elapsed = started.elapsed().as_secs_f64();

    if status.code() != Some(config.expected_exit_code) {
        return Err(BenchmarkError::UnexpectedExitCode {
            expected: config.expected_exit_code,
            actual: status.code(),
            iteration,
            warmup,
        });
    }

    Ok(elapsed)
}

fn redirect_stdio(redirect: &Redirect, input: bool) -> io::Result<Stdio> {
    match redirect {
        Redirect::Inherit => Ok(Stdio::inherit()),
        Redirect::Null => Ok(Stdio::null()),
        Redirect::File(path) if input => Ok(File::open(path)?.into()),
        Redirect::File(path) => Ok(File::create(path)?.into()),
    }
}

fn json_number(value: f64) -> String {
    assert!(value.is_finite(), "benchmark durations must be finite");
    value.to_string()
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{00}'..='\u{1f}' => {
                write!(escaped, "\\u{:04x}", character as u32).unwrap();
            }
            _ => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "kfrc-benchmark-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn computes_statistics_without_reordering_samples() {
        let samples = [4.0, 1.0, 3.0, 2.0];
        let statistics = summarize(&samples);

        assert_eq!(statistics.mean, 2.5);
        assert_eq!(statistics.median, 2.5);
        assert!((statistics.std_dev - (5.0_f64 / 3.0).sqrt()).abs() < 1e-12);
        assert_eq!(statistics.min, 1.0);
        assert_eq!(statistics.max, 4.0);
        assert_eq!(samples, [4.0, 1.0, 3.0, 2.0]);

        let single = summarize(&[1.5]);
        assert_eq!(single.median, 1.5);
        assert_eq!(single.std_dev, 0.0);
    }

    #[test]
    fn rejects_zero_timed_runs() {
        let mut config = BenchmarkConfig::new("unused");
        config.runs = 0;
        assert!(matches!(
            run_benchmark(&config),
            Err(BenchmarkError::ZeroRuns)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn runs_warmups_and_collects_ordered_samples() {
        let mut config = BenchmarkConfig::new("/bin/sh");
        config.args = vec!["-c".into(), "exit 0".into()];
        config.runs = 3;
        config.warmup_runs = 2;
        config.stdout = Redirect::Null;

        let result = run_benchmark(&config).unwrap();

        assert_eq!(result.samples.len(), 3);
        assert_eq!(result.warmup_runs, 2);
        assert!(result.samples.iter().all(|sample| *sample >= 0.0));
        assert_eq!(
            result.min,
            result.samples.iter().copied().fold(f64::INFINITY, f64::min)
        );
        assert_eq!(
            result.max,
            result.samples.iter().copied().fold(0.0, f64::max)
        );
        assert!((result.mean - result.samples.iter().sum::<f64>() / 3.0).abs() < f64::EPSILON);
    }

    #[cfg(unix)]
    #[test]
    fn validates_exit_codes_during_warmup_and_timed_runs() {
        let mut config = BenchmarkConfig::new("/bin/sh");
        config.args = vec!["-c".into(), "exit 7".into()];
        config.warmup_runs = 1;
        config.runs = 1;

        assert!(matches!(
            run_benchmark(&config),
            Err(BenchmarkError::UnexpectedExitCode {
                expected: 0,
                actual: Some(7),
                iteration: 0,
                warmup: true,
            })
        ));

        config.warmup_runs = 0;
        assert!(matches!(
            run_benchmark(&config),
            Err(BenchmarkError::UnexpectedExitCode {
                expected: 0,
                actual: Some(7),
                iteration: 0,
                warmup: false,
            })
        ));

        config.expected_exit_code = 7;
        assert_eq!(run_benchmark(&config).unwrap().samples.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn redirects_stdin_and_stdout_to_files() {
        let directory = TestDir::new();
        let input = directory.join("input.txt");
        let output = directory.join("output.txt");
        fs::write(&input, "input through cat\n").unwrap();
        fs::write(&output, "stale contents that must be truncated").unwrap();

        let mut config = BenchmarkConfig::new("/bin/cat");
        config.runs = 2;
        config.stdin = Redirect::File(input);
        config.stdout = Redirect::File(output.clone());

        run_benchmark(&config).unwrap();

        assert_eq!(fs::read_to_string(output).unwrap(), "input through cat\n");
    }

    #[cfg(unix)]
    #[test]
    fn supports_null_redirection() {
        let mut config = BenchmarkConfig::new("/bin/sh");
        config.args = vec!["-c".into(), "printf discarded; exit 0".into()];
        config.runs = 1;
        config.stdin = Redirect::Null;
        config.stdout = Redirect::Null;
        assert_eq!(run_benchmark(&config).unwrap().samples.len(), 1);
    }

    #[test]
    fn serializes_json_with_units_samples_and_escaped_strings() {
        let result = BenchmarkResult {
            command: "cmd\"\\\n\u{01}".into(),
            args: vec!["tab\tvalue".into(), "雪".into()],
            warmup_runs: 2,
            expected_exit_code: 0,
            samples: vec![0.25, 0.5],
            mean: 0.375,
            median: 0.375,
            std_dev: 0.0,
            min: 0.25,
            max: 0.5,
        };

        assert_eq!(
            result.to_json(),
            "{\"command\":\"cmd\\\"\\\\\\n\\u0001\",\"args\":[\"tab\\tvalue\",\"雪\"],\"runs\":2,\"warmup_runs\":2,\"expected_exit_code\":0,\"unit\":\"seconds\",\"mean\":0.375,\"median\":0.375,\"stddev\":0,\"min\":0.25,\"max\":0.5,\"samples\":[0.25,0.5]}"
        );
    }

    #[test]
    fn escapes_every_json_control_character() {
        let escaped = json_string("\u{0}\u{8}\u{c}\u{b}\u{1f}");
        assert_eq!(escaped, "\"\\u0000\\b\\f\\u000b\\u001f\"");
    }
}
