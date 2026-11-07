use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactMetadata {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified_unix_ms: Option<i64>,
    pub stdout_bytes: Option<u64>,
    pub stderr_bytes: Option<u64>,
    pub toolchain_fingerprint: String,
    pub command_fingerprint: String,
}

impl ArtifactMetadata {
    pub fn collect(
        path: impl AsRef<Path>,
        stdout_path: Option<&Path>,
        stderr_path: Option<&Path>,
        toolchain: &str,
        command: &str,
    ) -> io::Result<Self> {
        let path = path.as_ref();
        let metadata = fs::metadata(path)?;
        let stdout_bytes = stdout_path
            .map(|path| fs::metadata(path).map(|metadata| metadata.len()))
            .transpose()?;
        let stderr_bytes = stderr_path
            .map(|path| fs::metadata(path).map(|metadata| metadata.len()))
            .transpose()?;
        let modified_unix_ms = metadata.modified().ok().and_then(system_time_to_unix_ms);

        Ok(Self {
            path: path.to_path_buf(),
            size_bytes: metadata.len(),
            modified_unix_ms,
            stdout_bytes,
            stderr_bytes,
            toolchain_fingerprint: fingerprint(toolchain),
            command_fingerprint: fingerprint(command),
        })
    }

    pub fn to_json(&self) -> String {
        let mut json = String::with_capacity(256);
        json.push_str("{\"path\":");
        push_json_string(&mut json, &self.path.to_string_lossy());
        json.push_str(",\"size_bytes\":");
        json.push_str(&self.size_bytes.to_string());
        json.push_str(",\"modified_unix_ms\":");
        push_optional_number(&mut json, self.modified_unix_ms);
        json.push_str(",\"stdout_bytes\":");
        push_optional_number(&mut json, self.stdout_bytes);
        json.push_str(",\"stderr_bytes\":");
        push_optional_number(&mut json, self.stderr_bytes);
        json.push_str(",\"toolchain_fingerprint\":");
        push_json_string(&mut json, &self.toolchain_fingerprint);
        json.push_str(",\"command_fingerprint\":");
        push_json_string(&mut json, &self.command_fingerprint);
        json.push('}');
        json
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeComparison {
    pub baseline_bytes: u64,
    pub candidate_bytes: u64,
    pub size_ratio: Option<f64>,
    pub max_ratio: f64,
    pub non_regression: bool,
}

impl SizeComparison {
    pub fn new(baseline_bytes: u64, candidate_bytes: u64, max_ratio: f64) -> Self {
        Self {
            baseline_bytes,
            candidate_bytes,
            size_ratio: size_ratio(baseline_bytes, candidate_bytes),
            max_ratio,
            non_regression: meets_non_regression_threshold(
                baseline_bytes,
                candidate_bytes,
                max_ratio,
            ),
        }
    }

    pub fn to_json(&self) -> String {
        let mut json = String::with_capacity(160);
        json.push_str("{\"baseline_bytes\":");
        json.push_str(&self.baseline_bytes.to_string());
        json.push_str(",\"candidate_bytes\":");
        json.push_str(&self.candidate_bytes.to_string());
        json.push_str(",\"size_ratio\":");
        match self.size_ratio {
            Some(ratio) => push_finite_f64(&mut json, ratio),
            None => json.push_str("null"),
        }
        json.push_str(",\"max_ratio\":");
        push_finite_f64(&mut json, self.max_ratio);
        json.push_str(",\"non_regression\":");
        json.push_str(if self.non_regression { "true" } else { "false" });
        json.push('}');
        json
    }
}

pub fn collect_artifact(
    path: impl AsRef<Path>,
    stdout_path: Option<&Path>,
    stderr_path: Option<&Path>,
    toolchain: &str,
    command: &str,
) -> io::Result<ArtifactMetadata> {
    ArtifactMetadata::collect(path, stdout_path, stderr_path, toolchain, command)
}

pub fn compare_sizes(baseline_bytes: u64, candidate_bytes: u64, max_ratio: f64) -> SizeComparison {
    SizeComparison::new(baseline_bytes, candidate_bytes, max_ratio)
}

pub fn size_ratio(baseline_bytes: u64, candidate_bytes: u64) -> Option<f64> {
    (baseline_bytes != 0).then(|| candidate_bytes as f64 / baseline_bytes as f64)
}

pub fn meets_non_regression_threshold(
    baseline_bytes: u64,
    candidate_bytes: u64,
    max_ratio: f64,
) -> bool {
    if !max_ratio.is_finite() || max_ratio < 0.0 {
        return false;
    }
    if max_ratio == 1.0 {
        return candidate_bytes <= baseline_bytes;
    }
    candidate_bytes as f64 <= baseline_bytes as f64 * max_ratio
}

pub fn fingerprint(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn system_time_to_unix_ms(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).ok(),
        Err(error) => i64::try_from(error.duration().as_millis())
            .ok()
            .and_then(|milliseconds| milliseconds.checked_neg()),
    }
}

fn push_json_string(json: &mut String, value: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";

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
                let code = character as usize;
                json.push_str("\\u00");
                json.push(HEX[(code >> 4) & 0x0f] as char);
                json.push(HEX[code & 0x0f] as char);
            }
            _ => json.push(character),
        }
    }
    json.push('"');
}

fn push_optional_number(json: &mut String, value: Option<impl ToString>) {
    match value {
        Some(value) => json.push_str(&value.to_string()),
        None => json.push_str("null"),
    }
}

fn push_finite_f64(json: &mut String, value: f64) {
    if value.is_finite() {
        json.push_str(&value.to_string());
    } else {
        json.push_str("null");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!("kfrc-artifact-test-{}-{id}", process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn collects_artifact_and_optional_output_sizes() {
        let temp = TempDir::new();
        let artifact = temp.join("program");
        let stdout = temp.join("stdout.log");
        let stderr = temp.join("stderr.log");
        fs::write(&artifact, b"abcdef").unwrap();
        fs::write(&stdout, b"out").unwrap();
        fs::write(&stderr, b"").unwrap();

        let artifact = ArtifactMetadata::collect(
            &artifact,
            Some(stdout.as_path()),
            Some(stderr.as_path()),
            "rustc 1.85.0",
            "rustc --release",
        )
        .unwrap();

        assert_eq!(artifact.size_bytes, 6);
        assert!(artifact.modified_unix_ms.is_some());
        assert_eq!(artifact.stdout_bytes, Some(3));
        assert_eq!(artifact.stderr_bytes, Some(0));
        assert_eq!(artifact.toolchain_fingerprint, fingerprint("rustc 1.85.0"));
        assert_eq!(artifact.command_fingerprint, fingerprint("rustc --release"));
    }

    #[test]
    fn supports_absent_output_files_and_reports_missing_ones() {
        let temp = TempDir::new();
        let artifact = temp.join("program");
        fs::write(&artifact, b"x").unwrap();

        let metadata = collect_artifact(&artifact, None, None, "toolchain", "command").unwrap();
        assert_eq!(metadata.stdout_bytes, None);
        assert_eq!(metadata.stderr_bytes, None);
        assert!(metadata.to_json().contains("\"stdout_bytes\":null"));
        assert!(metadata.to_json().contains("\"stderr_bytes\":null"));

        let missing = temp.join("missing.log");
        assert!(
            ArtifactMetadata::collect(&artifact, Some(&missing), None, "toolchain", "command")
                .is_err()
        );
    }

    #[test]
    fn artifact_json_is_exact_and_escapes_control_characters() {
        let metadata = ArtifactMetadata {
            path: PathBuf::from("a\"b\\c\n\u{1f}"),
            size_bytes: 7,
            modified_unix_ms: Some(-2),
            stdout_bytes: None,
            stderr_bytes: Some(4),
            toolchain_fingerprint: "0123456789abcdef".into(),
            command_fingerprint: "fedcba9876543210".into(),
        };

        assert_eq!(
            metadata.to_json(),
            "{\"path\":\"a\\\"b\\\\c\\n\\u001f\",\"size_bytes\":7,\
             \"modified_unix_ms\":-2,\"stdout_bytes\":null,\"stderr_bytes\":4,\
             \"toolchain_fingerprint\":\"0123456789abcdef\",\
             \"command_fingerprint\":\"fedcba9876543210\"}"
        );
    }

    #[test]
    fn fingerprints_are_stable_and_input_sensitive() {
        assert_eq!(fingerprint(""), "cbf29ce484222325");
        assert_eq!(fingerprint("a"), "af63dc4c8601ec8c");
        assert_ne!(fingerprint("rustc -O"), fingerprint("rustc -C opt-level=3"));
    }

    #[test]
    fn ratio_is_candidate_divided_by_baseline() {
        assert_eq!(size_ratio(200, 100), Some(0.5));
        assert_eq!(size_ratio(100, 125), Some(1.25));
        assert_eq!(size_ratio(0, 0), None);
        assert_eq!(size_ratio(0, 1), None);
    }

    #[test]
    fn non_regression_uses_candidate_not_greater_than_allowed_size() {
        assert!(meets_non_regression_threshold(100, 100, 1.0));
        assert!(!meets_non_regression_threshold(100, 101, 1.0));
        assert!(meets_non_regression_threshold(100, 105, 1.05));
        assert!(!meets_non_regression_threshold(100, 106, 1.05));
        assert!(meets_non_regression_threshold(100, 90, 0.9));
        assert!(!meets_non_regression_threshold(100, 91, 0.9));
        assert!(meets_non_regression_threshold(100, 50, 1.0));
        assert!(meets_non_regression_threshold(u64::MAX, u64::MAX - 1, 1.0));
        assert!(!meets_non_regression_threshold(u64::MAX - 1, u64::MAX, 1.0));
    }

    #[test]
    fn zero_baseline_and_invalid_thresholds_are_well_defined() {
        assert!(meets_non_regression_threshold(0, 0, 1.0));
        assert!(!meets_non_regression_threshold(0, 1, 1.0));
        assert!(meets_non_regression_threshold(100, 0, 0.0));
        assert!(!meets_non_regression_threshold(100, 1, 0.0));
        assert!(!meets_non_regression_threshold(100, 100, -0.1));
        assert!(!meets_non_regression_threshold(100, 100, f64::NAN));
        assert!(!meets_non_regression_threshold(100, 100, f64::INFINITY));
    }

    #[test]
    fn comparison_json_contains_ratio_and_decision() {
        let comparison = compare_sizes(100, 105, 1.05);
        assert!(comparison.non_regression);
        assert_eq!(
            comparison.to_json(),
            "{\"baseline_bytes\":100,\"candidate_bytes\":105,\"size_ratio\":1.05,\
             \"max_ratio\":1.05,\"non_regression\":true}"
        );

        let zero_baseline = compare_sizes(0, 1, 1.0);
        assert_eq!(
            zero_baseline.to_json(),
            "{\"baseline_bytes\":0,\"candidate_bytes\":1,\"size_ratio\":null,\
             \"max_ratio\":1,\"non_regression\":false}"
        );

        let invalid = compare_sizes(1, 1, f64::NAN);
        assert!(invalid.to_json().contains("\"max_ratio\":null"));
        assert!(invalid.to_json().contains("\"non_regression\":false"));
    }

    #[test]
    fn modification_time_handles_pre_epoch_values() {
        assert_eq!(system_time_to_unix_ms(UNIX_EPOCH), Some(0));
        assert_eq!(
            system_time_to_unix_ms(UNIX_EPOCH + Duration::from_millis(12)),
            Some(12)
        );
        assert_eq!(
            system_time_to_unix_ms(UNIX_EPOCH - Duration::from_millis(2)),
            Some(-2)
        );
    }
}
