use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const AUDIT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseFilePolicy {
    pub expand: bool,
    pub allowed_roots: Vec<PathBuf>,
    pub max_bytes: u64,
    pub max_depth: usize,
}

impl ResponseFilePolicy {
    pub fn expanded_in(allowed_roots: Vec<PathBuf>) -> Self {
        Self {
            expand: true,
            allowed_roots,
            ..Self::default()
        }
    }
}

impl Default for ResponseFilePolicy {
    fn default() -> Self {
        Self {
            expand: false,
            allowed_roots: Vec::new(),
            max_bytes: 1024 * 1024,
            max_depth: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseFileStatus {
    PathOnly,
    Expanded,
    OutsideAllowedRoots,
    NotARegularFile,
    TooLarge,
    DepthLimit,
    Cycle,
    InvalidEncoding,
    Io(io::ErrorKind),
}

impl ResponseFileStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::PathOnly => "path_only",
            Self::Expanded => "expanded",
            Self::OutsideAllowedRoots => "outside_allowed_roots",
            Self::NotARegularFile => "not_a_regular_file",
            Self::TooLarge => "too_large",
            Self::DepthLimit => "depth_limit",
            Self::Cycle => "cycle",
            Self::InvalidEncoding => "invalid_encoding",
            Self::Io(_) => "io_error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseFileAudit {
    pub path: PathBuf,
    pub depth: usize,
    pub status: ResponseFileStatus,
    pub content: Option<Vec<u8>>,
    pub expanded_args: Option<Vec<OsString>>,
}

impl ResponseFileAudit {
    fn new(path: PathBuf, depth: usize) -> Self {
        Self {
            path,
            depth,
            status: ResponseFileStatus::PathOnly,
            content: None,
            expanded_args: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timestamp {
    pub unix_seconds: i64,
    pub nanoseconds: u32,
}

impl Timestamp {
    pub const fn from_unix_seconds(unix_seconds: i64) -> Self {
        Self {
            unix_seconds,
            nanoseconds: 0,
        }
    }

    pub fn now() -> Self {
        Self::from(SystemTime::now())
    }

    pub fn rfc3339(self) -> String {
        let days = self.unix_seconds.div_euclid(86_400);
        let seconds_of_day = self.unix_seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        let hour = seconds_of_day / 3_600;
        let minute = (seconds_of_day % 3_600) / 60;
        let second = seconds_of_day % 60;
        format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:09}Z",
            self.nanoseconds
        )
    }
}

impl From<SystemTime> for Timestamp {
    fn from(time: SystemTime) -> Self {
        match time.duration_since(UNIX_EPOCH) {
            Ok(duration) => Self {
                unix_seconds: i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
                nanoseconds: duration.subsec_nanos(),
            },
            Err(error) => {
                let duration = error.duration();
                let seconds = i64::try_from(duration.as_secs()).unwrap_or(i64::MAX);
                if duration.subsec_nanos() == 0 {
                    Self {
                        unix_seconds: -seconds,
                        nanoseconds: 0,
                    }
                } else {
                    Self {
                        unix_seconds: seconds.saturating_neg().saturating_sub(1),
                        nanoseconds: 1_000_000_000 - duration.subsec_nanos(),
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditRecord {
    pub schema_version: u32,
    pub crate_name: Option<OsString>,
    pub rustc: PathBuf,
    pub args: Vec<OsString>,
    pub backend: String,
    pub profile: String,
    pub pid: u32,
    pub timestamp: Timestamp,
    pub response_files: Vec<ResponseFileAudit>,
}

impl AuditRecord {
    pub fn new<P, I, S>(
        rustc: P,
        args: I,
        backend: impl Into<String>,
        profile: impl Into<String>,
    ) -> Self
    where
        P: Into<PathBuf>,
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self::new_with_response_policy(
            rustc,
            args,
            backend,
            profile,
            &ResponseFilePolicy::default(),
        )
    }

    pub fn new_with_response_policy<P, I, S>(
        rustc: P,
        args: I,
        backend: impl Into<String>,
        profile: impl Into<String>,
        response_policy: &ResponseFilePolicy,
    ) -> Self
    where
        P: Into<PathBuf>,
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
        let (response_files, effective_args) = inspect_arguments(&args, response_policy);
        Self {
            schema_version: AUDIT_SCHEMA_VERSION,
            crate_name: crate_name(&effective_args),
            rustc: rustc.into(),
            args,
            backend: backend.into(),
            profile: profile.into(),
            pid: std::process::id(),
            timestamp: Timestamp::now(),
            response_files,
        }
    }

    pub fn to_json_line(&self) -> String {
        let mut json = String::new();
        json.push('{');
        write!(json, "\"schema_version\":{}", self.schema_version).unwrap();
        json.push_str(",\"crate_name\":");
        match &self.crate_name {
            Some(name) => push_os_string(&mut json, name),
            None => json.push_str("null"),
        }
        json.push_str(",\"rustc\":");
        push_os_string(&mut json, self.rustc.as_os_str());
        json.push_str(",\"args\":[");
        for (index, arg) in self.args.iter().enumerate() {
            if index != 0 {
                json.push(',');
            }
            push_os_string(&mut json, arg);
        }
        json.push_str("],\"backend\":");
        push_json_string(&mut json, &self.backend);
        json.push_str(",\"profile\":");
        push_json_string(&mut json, &self.profile);
        write!(json, ",\"pid\":{}", self.pid).unwrap();
        write!(
            json,
            ",\"timestamp\":{{\"unix_seconds\":{},\"nanoseconds\":{},\"rfc3339\":",
            self.timestamp.unix_seconds, self.timestamp.nanoseconds
        )
        .unwrap();
        push_json_string(&mut json, &self.timestamp.rfc3339());
        json.push_str("},\"response_files\":[");
        for (index, response) in self.response_files.iter().enumerate() {
            if index != 0 {
                json.push(',');
            }
            push_response_file(&mut json, response);
        }
        json.push_str("]}");
        json
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditLog {
    directory: PathBuf,
    response_policy: ResponseFilePolicy,
}

impl AuditLog {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            response_policy: ResponseFilePolicy::default(),
        }
    }

    pub fn with_response_policy(mut self, response_policy: ResponseFilePolicy) -> Self {
        self.response_policy = response_policy;
        self
    }

    pub fn response_policy(&self) -> &ResponseFilePolicy {
        &self.response_policy
    }

    pub fn record<P, I, S>(
        &self,
        rustc: P,
        args: I,
        backend: impl Into<String>,
        profile: impl Into<String>,
    ) -> AuditRecord
    where
        P: Into<PathBuf>,
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        AuditRecord::new_with_response_policy(rustc, args, backend, profile, &self.response_policy)
    }

    pub fn log<P, I, S>(
        &self,
        rustc: P,
        args: I,
        backend: impl Into<String>,
        profile: impl Into<String>,
    ) -> io::Result<PathBuf>
    where
        P: Into<PathBuf>,
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let record = self.record(rustc, args, backend, profile);
        self.append(&record)
    }

    pub fn path_for(&self, crate_name: Option<&OsStr>) -> PathBuf {
        self.directory
            .join(format!("{}.jsonl", crate_file_stem(crate_name)))
    }

    /// Appends one complete JSON line. On Linux, O_APPEND and one write syscall
    /// keep offset selection and each concurrent regular-file write atomic.
    pub fn append(&self, record: &AuditRecord) -> io::Result<PathBuf> {
        fs::create_dir_all(&self.directory)?;
        let path = self.path_for(record.crate_name.as_deref());
        let mut line = record.to_json_line();
        line.push('\n');
        append_json_line(&path, line.as_bytes())?;
        Ok(path)
    }
}

pub(crate) fn append_json_line(path: &Path, line: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;

    loop {
        match file.write(line) {
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

fn inspect_arguments(
    args: &[OsString],
    policy: &ResponseFilePolicy,
) -> (Vec<ResponseFileAudit>, Vec<OsString>) {
    let mut response_files = Vec::new();
    let mut effective_args = Vec::with_capacity(args.len());
    let mut stack = HashSet::new();

    for arg in args {
        if let Some(path) = response_file_path(arg) {
            if policy.expand {
                if let Some(expanded) =
                    inspect_response_file(path, 0, policy, &mut stack, &mut response_files)
                {
                    effective_args.extend(expanded);
                } else {
                    effective_args.push(arg.clone());
                }
            } else {
                response_files.push(ResponseFileAudit::new(path, 0));
                effective_args.push(arg.clone());
            }
        } else {
            effective_args.push(arg.clone());
        }
    }

    (response_files, effective_args)
}

fn inspect_response_file(
    path: PathBuf,
    depth: usize,
    policy: &ResponseFilePolicy,
    stack: &mut HashSet<PathBuf>,
    audits: &mut Vec<ResponseFileAudit>,
) -> Option<Vec<OsString>> {
    let mut audit = ResponseFileAudit::new(path.clone(), depth);
    let result = (|| {
        if depth >= policy.max_depth {
            audit.status = ResponseFileStatus::DepthLimit;
            return None;
        }

        // Roots must be trusted. Canonical containment prevents traversal and
        // symlink escapes; std alone cannot defend against concurrent root rewrites.
        let canonical = match fs::canonicalize(&path) {
            Ok(path) => path,
            Err(error) => {
                audit.status = ResponseFileStatus::Io(error.kind());
                return None;
            }
        };
        let allowed = policy
            .allowed_roots
            .iter()
            .any(|root| fs::canonicalize(root).is_ok_and(|root| canonical.starts_with(root)));
        if !allowed {
            audit.status = ResponseFileStatus::OutsideAllowedRoots;
            return None;
        }
        if !stack.insert(canonical.clone()) {
            audit.status = ResponseFileStatus::Cycle;
            return None;
        }

        let result = (|| {
            let file = match File::open(&canonical) {
                Ok(file) => file,
                Err(error) => {
                    audit.status = ResponseFileStatus::Io(error.kind());
                    return None;
                }
            };
            let metadata = match file.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    audit.status = ResponseFileStatus::Io(error.kind());
                    return None;
                }
            };
            if !metadata.is_file() {
                audit.status = ResponseFileStatus::NotARegularFile;
                return None;
            }
            if metadata.len() > policy.max_bytes {
                audit.status = ResponseFileStatus::TooLarge;
                return None;
            }

            let mut content = Vec::new();
            if let Err(error) = file
                .take(policy.max_bytes.saturating_add(1))
                .read_to_end(&mut content)
            {
                audit.status = ResponseFileStatus::Io(error.kind());
                return None;
            }
            if content.len() as u64 > policy.max_bytes {
                audit.status = ResponseFileStatus::TooLarge;
                return None;
            }
            audit.content = Some(content.clone());
            let Some(expanded_args) = parse_response_file(&content) else {
                audit.status = ResponseFileStatus::InvalidEncoding;
                return None;
            };
            audit.expanded_args = Some(expanded_args.clone());
            audit.status = ResponseFileStatus::Expanded;

            let mut effective_args = Vec::with_capacity(expanded_args.len());
            for arg in expanded_args {
                if let Some(nested_path) = response_file_path(&arg) {
                    if let Some(nested) =
                        inspect_response_file(nested_path, depth + 1, policy, stack, audits)
                    {
                        effective_args.extend(nested);
                    } else {
                        effective_args.push(arg);
                    }
                } else {
                    effective_args.push(arg);
                }
            }
            Some(effective_args)
        })();

        stack.remove(&canonical);
        result
    })();
    audits.push(audit);
    result
}

fn response_file_path(arg: &OsStr) -> Option<PathBuf> {
    let bytes = arg.as_encoded_bytes();
    if bytes.len() < 2 || bytes[0] != b'@' {
        return None;
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(OsString::from_vec(bytes[1..].to_vec())))
    }
    #[cfg(not(unix))]
    {
        arg.to_str()
            .and_then(|arg| arg.strip_prefix('@'))
            .map(PathBuf::from)
    }
}

fn parse_response_file(content: &[u8]) -> Option<Vec<OsString>> {
    let content = std::str::from_utf8(content).ok()?;
    Some(content.lines().map(OsString::from).collect())
}

fn crate_name(args: &[OsString]) -> Option<OsString> {
    let mut crate_name = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.to_str() {
            Some("--") => break,
            Some("--crate-name") => {
                if let Some(value) = args.get(index + 1) {
                    crate_name = Some(value.clone());
                    index += 1;
                }
            }
            Some(arg) => {
                if let Some(value) = arg.strip_prefix("--crate-name=") {
                    crate_name = Some(OsString::from(value));
                }
            }
            None => {}
        }
        index += 1;
    }
    crate_name
}

fn crate_file_stem(crate_name: Option<&OsStr>) -> String {
    let Some(crate_name) = crate_name else {
        return "_unknown".to_owned();
    };
    let bytes = crate_name.as_encoded_bytes();
    let mut encoded = String::new();
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-') {
            encoded.push(char::from(*byte));
        } else {
            write!(encoded, "_{byte:02x}").unwrap();
        }
    }
    if encoded.len() <= 160 {
        return format!("crate-{encoded}");
    }

    let first = fnv1a(bytes, 0xcbf29ce484222325);
    let second = fnv1a(bytes, 0x84222325cbf29ce4);
    format!("crate-{}-{first:016x}{second:016x}", &encoded[..48])
}

fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn push_response_file(json: &mut String, response: &ResponseFileAudit) {
    json.push_str("{\"path\":");
    push_os_string(json, response.path.as_os_str());
    write!(json, ",\"depth\":{}", response.depth).unwrap();
    json.push_str(",\"status\":");
    push_json_string(json, response.status.as_str());
    if let ResponseFileStatus::Io(kind) = response.status {
        json.push_str(",\"error_kind\":");
        push_json_string(json, &format!("{kind:?}"));
    }
    json.push_str(",\"content\":");
    match &response.content {
        Some(content) => push_bytes(json, content),
        None => json.push_str("null"),
    }
    json.push_str(",\"expanded_args\":");
    match &response.expanded_args {
        Some(args) => {
            json.push('[');
            for (index, arg) in args.iter().enumerate() {
                if index != 0 {
                    json.push(',');
                }
                push_os_string(json, arg);
            }
            json.push(']');
        }
        None => json.push_str("null"),
    }
    json.push('}');
}

fn push_os_string(json: &mut String, value: &OsStr) {
    match value.to_str() {
        Some(value) => push_encoded(json, "utf-8", |json| push_json_string(json, value)),
        None => push_encoded(json, "hex", |json| push_hex(json, value.as_encoded_bytes())),
    }
}

fn push_bytes(json: &mut String, value: &[u8]) {
    match std::str::from_utf8(value) {
        Ok(value) => push_encoded(json, "utf-8", |json| push_json_string(json, value)),
        Err(_) => push_encoded(json, "hex", |json| push_hex(json, value)),
    }
}

fn push_encoded(json: &mut String, encoding: &str, write_value: impl FnOnce(&mut String)) {
    json.push_str("{\"encoding\":");
    push_json_string(json, encoding);
    json.push_str(",\"value\":");
    write_value(json);
    json.push('}');
}

fn push_hex(json: &mut String, bytes: &[u8]) {
    json.push('"');
    for byte in bytes {
        write!(json, "{byte:02x}").unwrap();
    }
    json.push('"');
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
                write!(json, "\\u{:04x}", u32::from(character)).unwrap();
            }
            _ => json.push(character),
        }
    }
    json.push('"');
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_epoch + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::collections::BTreeSet;
    use std::env;
    #[cfg(target_os = "linux")]
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "kfrc-audit-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn strings(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn records_all_invocation_metadata_and_escapes_json() {
        let record = AuditRecord::new(
            "/toolchain/bin/rustc",
            strings(&[
                "--crate-name",
                "demo",
                "--emit=metadata",
                "quote\"\\\n\r\t\u{1}",
            ]),
            "llvm\"backend",
            "speed\nprofile",
        );
        let json = record.to_json_line();

        assert_eq!(record.schema_version, AUDIT_SCHEMA_VERSION);
        assert_eq!(record.crate_name, Some(OsString::from("demo")));
        assert_eq!(record.rustc, PathBuf::from("/toolchain/bin/rustc"));
        assert_eq!(record.pid, std::process::id());
        assert!(!json.contains('\n'));
        assert!(json.contains("\"crate_name\":{\"encoding\":\"utf-8\",\"value\":\"demo\"}"));
        assert!(json.contains("quote\\\"\\\\\\n\\r\\t\\u0001"));
        assert!(json.contains("\"backend\":\"llvm\\\"backend\""));
        assert!(json.contains("\"profile\":\"speed\\nprofile\""));
        assert!(json.contains("\"timestamp\":{"));
        assert!(json.contains("\"rfc3339\":"));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_arguments_and_paths_are_lossless_hex() {
        use std::os::unix::ffi::OsStringExt;

        let rustc = OsString::from_vec(vec![b'r', b'u', b's', b't', b'c', 0xff]);
        let arg = OsString::from_vec(vec![b'a', 0, 0x80, 0xff]);
        let record = AuditRecord::new(
            PathBuf::from(rustc),
            vec![OsString::from("--crate-name=demo"), arg],
            "llvm",
            "controlled",
        );
        let json = record.to_json_line();

        assert!(json.contains("\"rustc\":{\"encoding\":\"hex\",\"value\":\"7275737463ff\"}"));
        assert!(json.contains("{\"encoding\":\"hex\",\"value\":\"610080ff\"}"));
        assert!(!json.contains('\u{fffd}'));
    }

    #[test]
    fn appends_one_json_line_per_crate_without_path_traversal() {
        let temp = TempDir::new();
        let log = AuditLog::new(temp.path());
        let first = AuditRecord::new(
            "rustc",
            strings(&["--crate-name=first", "src/lib.rs"]),
            "llvm",
            "controlled",
        );
        let second = AuditRecord::new("rustc", strings(&["--crate-name=first"]), "llvm", "speed");
        let hostile = AuditRecord::new(
            "rustc",
            strings(&["--crate-name=../../escape"]),
            "llvm",
            "controlled",
        );

        let first_path = log.append(&first).unwrap();
        assert_eq!(log.append(&second).unwrap(), first_path);
        let hostile_path = log.append(&hostile).unwrap();
        let unknown_path = log
            .append(&AuditRecord::new(
                "rustc",
                Vec::<OsString>::new(),
                "llvm",
                "controlled",
            ))
            .unwrap();

        assert_eq!(fs::read_to_string(&first_path).unwrap().lines().count(), 2);
        assert_eq!(first_path.file_name().unwrap(), "crate-first.jsonl");
        assert_eq!(
            hostile_path.file_name().unwrap(),
            "crate-_2e_2e_2f_2e_2e_2fescape.jsonl"
        );
        assert_eq!(unknown_path.file_name().unwrap(), "_unknown.jsonl");
        assert!(hostile_path.starts_with(temp.path()));
        assert!(!temp.path().join("escape.jsonl").exists());
    }

    #[test]
    fn response_files_are_path_only_by_default() {
        let temp = TempDir::new();
        let response = temp.path().join("args.rsp");
        fs::write(&response, "--crate-name secret_crate\n").unwrap();
        let arg = OsString::from(format!("@{}", response.display()));
        let record = AuditRecord::new("rustc", vec![arg], "llvm", "controlled");

        assert_eq!(record.crate_name, None);
        assert_eq!(record.response_files.len(), 1);
        assert_eq!(
            record.response_files[0].status,
            ResponseFileStatus::PathOnly
        );
        assert_eq!(record.response_files[0].content, None);
        assert!(!record.to_json_line().contains("secret_crate"));
    }

    #[test]
    fn expansion_is_limited_to_allowed_roots_and_extracts_crate_name() {
        let temp = TempDir::new();
        let allowed = temp.path().join("allowed");
        fs::create_dir(&allowed).unwrap();
        let response = allowed.join("args.rsp");
        fs::write(&response, "--crate-name\nfrom_response\n--emit\nmetadata\n").unwrap();
        let outside = temp.path().join("outside.rsp");
        fs::write(&outside, "DO_NOT_READ_THIS_SECRET").unwrap();
        let policy = ResponseFilePolicy::expanded_in(vec![allowed]);
        let args = vec![
            OsString::from(format!("@{}", response.display())),
            OsString::from(format!("@{}", outside.display())),
        ];

        let record = AuditRecord::new_with_response_policy("rustc", args, "llvm", "size", &policy);

        assert_eq!(record.crate_name, Some(OsString::from("from_response")));
        assert_eq!(record.response_files.len(), 2);
        let expanded = record
            .response_files
            .iter()
            .find(|response| response.status == ResponseFileStatus::Expanded)
            .unwrap();
        assert_eq!(
            expanded.expanded_args,
            Some(strings(&[
                "--crate-name",
                "from_response",
                "--emit",
                "metadata"
            ]))
        );
        let denied = record
            .response_files
            .iter()
            .find(|response| response.status == ResponseFileStatus::OutsideAllowedRoots)
            .unwrap();
        assert_eq!(denied.content, None);
        assert!(!record.to_json_line().contains("DO_NOT_READ_THIS_SECRET"));
    }

    #[test]
    fn response_expansion_rejects_traversal_symlinks_cycles_and_large_files() {
        let temp = TempDir::new();
        let allowed = temp.path().join("allowed");
        fs::create_dir(&allowed).unwrap();
        let outside = temp.path().join("outside.rsp");
        fs::write(&outside, "outside-secret").unwrap();
        let traversal = OsString::from(format!("@{}/../outside.rsp", allowed.display()));
        let policy = ResponseFilePolicy {
            expand: true,
            allowed_roots: vec![allowed.clone()],
            max_bytes: 8,
            max_depth: 8,
        };
        let record = AuditRecord::new_with_response_policy(
            "rustc",
            vec![traversal],
            "llvm",
            "controlled",
            &policy,
        );
        assert_eq!(
            record.response_files[0].status,
            ResponseFileStatus::OutsideAllowedRoots
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = allowed.join("link.rsp");
            symlink(&outside, &link).unwrap();
            let record = AuditRecord::new_with_response_policy(
                "rustc",
                vec![OsString::from(format!("@{}", link.display()))],
                "llvm",
                "controlled",
                &policy,
            );
            assert_eq!(
                record.response_files[0].status,
                ResponseFileStatus::OutsideAllowedRoots
            );
            assert_eq!(record.response_files[0].content, None);
        }

        let large = allowed.join("large.rsp");
        fs::write(&large, "more-than-eight-bytes").unwrap();
        let record = AuditRecord::new_with_response_policy(
            "rustc",
            vec![OsString::from(format!("@{}", large.display()))],
            "llvm",
            "controlled",
            &policy,
        );
        assert_eq!(
            record.response_files[0].status,
            ResponseFileStatus::TooLarge
        );

        let cycle = allowed.join("cycle.rsp");
        fs::write(&cycle, format!("@{}\n", cycle.display())).unwrap();
        let policy = ResponseFilePolicy {
            max_bytes: 1024,
            ..policy
        };
        let record = AuditRecord::new_with_response_policy(
            "rustc",
            vec![OsString::from(format!("@{}", cycle.display()))],
            "llvm",
            "controlled",
            &policy,
        );
        assert!(
            record
                .response_files
                .iter()
                .any(|response| response.status == ResponseFileStatus::Cycle)
        );
    }

    #[test]
    fn nested_response_files_expand_in_argument_order() {
        let temp = TempDir::new();
        let nested = temp.path().join("nested.rsp");
        let outer = temp.path().join("outer.rsp");
        fs::write(&nested, "nested_crate\n").unwrap();
        fs::write(&outer, format!("--crate-name\n@{}\n", nested.display())).unwrap();
        let policy = ResponseFilePolicy::expanded_in(vec![temp.path().to_path_buf()]);
        let record = AuditRecord::new_with_response_policy(
            "rustc",
            vec![OsString::from(format!("@{}", outer.display()))],
            "llvm",
            "controlled",
            &policy,
        );

        assert_eq!(record.crate_name, Some(OsString::from("nested_crate")));
        assert_eq!(record.response_files.len(), 2);
        assert!(
            record
                .response_files
                .iter()
                .all(|response| response.status == ResponseFileStatus::Expanded)
        );
    }

    #[test]
    fn response_files_preserve_empty_options_and_crlf() {
        let temp = TempDir::new();
        let response = temp.path().join("empty-options.rsp");
        fs::write(&response, "first\r\n\r\nthird\r\n").unwrap();
        let policy = ResponseFilePolicy::expanded_in(vec![temp.path().to_path_buf()]);
        let record = AuditRecord::new_with_response_policy(
            "rustc",
            vec![OsString::from(format!("@{}", response.display()))],
            "llvm",
            "controlled",
            &policy,
        );

        assert_eq!(
            record.response_files[0].expanded_args,
            Some(strings(&["first", "", "third"]))
        );
    }

    #[test]
    fn non_utf8_response_content_is_audited_without_loss() {
        let temp = TempDir::new();
        let response = temp.path().join("non-utf8.rsp");
        fs::write(&response, [b'-', b'-', b'f', b'o', b'o', b'\n', 0xff]).unwrap();
        let policy = ResponseFilePolicy::expanded_in(vec![temp.path().to_path_buf()]);
        let record = AuditRecord::new_with_response_policy(
            "rustc",
            vec![OsString::from(format!("@{}", response.display()))],
            "llvm",
            "controlled",
            &policy,
        );

        assert_eq!(
            record.response_files[0].status,
            ResponseFileStatus::InvalidEncoding
        );
        assert_eq!(record.response_files[0].expanded_args, None);
        assert_eq!(
            record.response_files[0].content,
            Some(vec![b'-', b'-', b'f', b'o', b'o', b'\n', 0xff])
        );
        assert!(
            record
                .to_json_line()
                .contains("\"value\":\"2d2d666f6f0aff\"")
        );
    }

    #[test]
    fn timestamps_are_unix_based_and_rfc3339() {
        assert_eq!(Timestamp::from(UNIX_EPOCH), Timestamp::from_unix_seconds(0));
        assert_eq!(
            Timestamp::from_unix_seconds(0).rfc3339(),
            "1970-01-01T00:00:00.000000000Z"
        );
        assert_eq!(
            Timestamp::from_unix_seconds(951_782_400).rfc3339(),
            "2000-02-29T00:00:00.000000000Z"
        );
        assert_eq!(
            Timestamp::from(UNIX_EPOCH - std::time::Duration::new(0, 1)),
            Timestamp {
                unix_seconds: -1,
                nanoseconds: 999_999_999,
            }
        );
        assert_eq!(
            Timestamp {
                unix_seconds: -1,
                nanoseconds: 999_999_999,
            }
            .rfc3339(),
            "1969-12-31T23:59:59.999999999Z"
        );
    }

    #[cfg(target_os = "linux")]
    fn concurrent_line(writer: usize, sequence: usize) -> String {
        format!(
            "{{\"writer\":{writer},\"sequence\":{sequence},\"padding\":\"{}\"}}\n",
            "x".repeat(256)
        )
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn external_append_writer() {
        let Ok(path) = env::var("KFRC_AUDIT_TEST_LOG") else {
            return;
        };
        let writer = env::var("KFRC_AUDIT_TEST_WRITER")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        for sequence in 0..100 {
            append_json_line(
                Path::new(&path),
                concurrent_line(writer, sequence).as_bytes(),
            )
            .unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_o_append_keeps_concurrent_process_lines_intact() {
        let temp = TempDir::new();
        let path = temp.path().join("shared.jsonl");
        let executable = env::current_exe().unwrap();
        let mut children = Vec::new();
        for writer in 0..4 {
            children.push(
                Command::new(&executable)
                    .args([
                        "--exact",
                        "audit::tests::external_append_writer",
                        "--nocapture",
                    ])
                    .env("KFRC_AUDIT_TEST_LOG", &path)
                    .env("KFRC_AUDIT_TEST_WRITER", writer.to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
        }
        for mut child in children {
            assert!(child.wait().unwrap().success());
        }

        let content = fs::read_to_string(&path).unwrap();
        let actual = content.lines().collect::<BTreeSet<_>>();
        let expected = (0..4)
            .flat_map(|writer| (0..100).map(move |sequence| (writer, sequence)))
            .map(|(writer, sequence)| {
                let line = concurrent_line(writer, sequence);
                line.strip_suffix('\n').unwrap().to_owned()
            })
            .collect::<BTreeSet<_>>();
        let actual = actual
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        assert_eq!(content.lines().count(), 400);
        assert!(content.ends_with('\n'));
    }
}
