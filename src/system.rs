use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub cgroup: CgroupInfo,
    pub rustc_vv: CommandOutput,
    pub git_version: CommandOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuInfo {
    pub model: Option<String>,
    pub logical_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryInfo {
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupInfo {
    pub cpu_quota: Option<CgroupCpuQuota>,
    pub cpuset: Option<CgroupCpuset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgroupVersion {
    V1,
    V2,
}

impl CgroupVersion {
    fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupCpuQuota {
    pub version: CgroupVersion,
    pub quota_us: Option<u64>,
    pub period_us: Option<u64>,
    pub unlimited: Option<bool>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupCpuset {
    pub version: CgroupVersion,
    pub cpus: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub command: String,
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
}

impl SystemInfo {
    pub fn collect() -> Self {
        let (model, proc_logical_count) = if cfg!(target_os = "linux") {
            fs::read_to_string("/proc/cpuinfo")
                .ok()
                .map(|contents| parse_proc_cpuinfo(&contents))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
        let logical_count = proc_logical_count.or_else(|| {
            std::thread::available_parallelism()
                .ok()
                .map(std::num::NonZero::get)
        });
        let total_bytes = if cfg!(target_os = "linux") {
            fs::read_to_string("/proc/meminfo")
                .ok()
                .and_then(|contents| parse_proc_meminfo(&contents))
        } else {
            None
        };

        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            cpu: CpuInfo {
                model,
                logical_count,
            },
            memory: MemoryInfo { total_bytes },
            cgroup: collect_cgroup(Path::new("/proc"), Path::new("/sys")),
            rustc_vv: run_command("rustc -Vv", "rustc", &["-Vv"]),
            git_version: run_command("git --version", "git", &["--version"]),
        }
    }

    pub fn to_json(&self) -> String {
        let mut json = String::with_capacity(1024);
        json.push_str("{\"os\":");
        push_json_string(&mut json, &self.os);
        json.push_str(",\"arch\":");
        push_json_string(&mut json, &self.arch);
        json.push_str(",\"cpu\":{\"model\":");
        push_optional_string(&mut json, self.cpu.model.as_deref());
        json.push_str(",\"logical_count\":");
        push_optional_number(&mut json, self.cpu.logical_count);
        json.push_str("},\"memory\":{\"total_bytes\":");
        push_optional_number(&mut json, self.memory.total_bytes);
        json.push_str("},\"cgroup\":{\"cpu_quota\":");
        push_cpu_quota_json(&mut json, self.cgroup.cpu_quota.as_ref());
        json.push_str(",\"cpuset\":");
        push_cpuset_json(&mut json, self.cgroup.cpuset.as_ref());
        json.push_str("},\"rustc_vv\":");
        push_command_json(&mut json, &self.rustc_vv);
        json.push_str(",\"git_version\":");
        push_command_json(&mut json, &self.git_version);
        json.push('}');
        json
    }
}

pub fn collect_system_info() -> SystemInfo {
    SystemInfo::collect()
}

fn run_command(display: &str, program: &str, args: &[&str]) -> CommandOutput {
    match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) => CommandOutput {
            command: display.to_owned(),
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            error: None,
        },
        Err(error) => CommandOutput {
            command: display.to_owned(),
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(error.to_string()),
        },
    }
}

fn parse_proc_cpuinfo(contents: &str) -> (Option<String>, Option<usize>) {
    let mut model = None;
    let mut fallback_model = None;
    let mut logical_count = 0_usize;

    for line in contents.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key == "processor" && value.parse::<usize>().is_ok() {
            logical_count += 1;
        } else if key == "model name" && !value.is_empty() {
            if model.is_none() {
                model = Some(value.to_owned());
            }
        } else if fallback_model.is_none()
            && (key.eq_ignore_ascii_case("model") || key.eq_ignore_ascii_case("cpu model"))
            && !value.is_empty()
        {
            fallback_model = Some(value.to_owned());
        }
    }

    (
        model.or(fallback_model),
        (logical_count != 0).then_some(logical_count),
    )
}

fn parse_proc_meminfo(contents: &str) -> Option<u64> {
    for line in contents.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() != "MemTotal" {
            continue;
        }
        let mut fields = value.split_whitespace();
        let kibibytes = fields.next()?.trim().parse::<u64>().ok()?;
        if !fields
            .next()
            .is_some_and(|unit| unit.eq_ignore_ascii_case("kb"))
        {
            return None;
        }
        return kibibytes.checked_mul(1024);
    }
    None
}

#[derive(Default)]
struct CgroupMembership {
    v2: Option<String>,
    cpu_v1: Option<String>,
    cpuset_v1: Option<String>,
}

#[derive(Default)]
struct CgroupRoots {
    v2: Vec<PathBuf>,
    cpu_v1: Vec<PathBuf>,
    cpuset_v1: Vec<PathBuf>,
}

fn collect_cgroup(proc_root: &Path, sys_root: &Path) -> CgroupInfo {
    let roots = discover_cgroup_roots(proc_root, sys_root);
    let cpu_quota = roots
        .v2
        .iter()
        .find_map(|root| read_v2_cpu_quota(root))
        .or_else(|| roots.cpu_v1.iter().find_map(|root| read_v1_cpu_quota(root)));
    let cpuset = roots
        .v2
        .iter()
        .find_map(|root| read_v2_cpuset(root))
        .or_else(|| roots.cpuset_v1.iter().find_map(|root| read_v1_cpuset(root)));

    CgroupInfo { cpu_quota, cpuset }
}

fn discover_cgroup_roots(proc_root: &Path, sys_root: &Path) -> CgroupRoots {
    let membership = fs::read_to_string(proc_root.join("self/cgroup"))
        .ok()
        .map(|contents| parse_cgroup_membership(&contents))
        .unwrap_or_default();
    let mut roots = CgroupRoots::default();

    if let Ok(contents) = fs::read_to_string(proc_root.join("self/mountinfo")) {
        add_mountinfo_roots(&contents, &membership, &mut roots);
    }

    let base = sys_root.join("fs/cgroup");
    push_cgroup_root(
        &mut roots.v2,
        &base,
        membership.v2.as_deref().unwrap_or("/"),
    );
    for name in ["cpu", "cpu,cpuacct", "cpuacct,cpu"] {
        push_cgroup_root(
            &mut roots.cpu_v1,
            &base.join(name),
            membership.cpu_v1.as_deref().unwrap_or("/"),
        );
    }
    push_cgroup_root(
        &mut roots.cpuset_v1,
        &base.join("cpuset"),
        membership.cpuset_v1.as_deref().unwrap_or("/"),
    );
    roots
}

fn parse_cgroup_membership(contents: &str) -> CgroupMembership {
    let mut membership = CgroupMembership::default();
    for line in contents.lines() {
        let mut fields = line.splitn(3, ':');
        let Some(hierarchy) = fields.next() else {
            continue;
        };
        let Some(controllers) = fields.next() else {
            continue;
        };
        let Some(path) = fields.next() else {
            continue;
        };
        if hierarchy == "0" && controllers.is_empty() && membership.v2.is_none() {
            membership.v2 = Some(path.to_owned());
        }
        for controller in controllers.split(',') {
            match controller {
                "cpu" if membership.cpu_v1.is_none() => {
                    membership.cpu_v1 = Some(path.to_owned());
                }
                "cpuset" if membership.cpuset_v1.is_none() => {
                    membership.cpuset_v1 = Some(path.to_owned());
                }
                _ => {}
            }
        }
    }
    membership
}

fn add_mountinfo_roots(contents: &str, membership: &CgroupMembership, roots: &mut CgroupRoots) {
    for line in contents.lines() {
        let Some((before, after)) = line.split_once(" - ") else {
            continue;
        };
        let fields = before.split_whitespace().collect::<Vec<_>>();
        let Some(mount_point) = fields.get(4) else {
            continue;
        };
        let mount_point = PathBuf::from(decode_mount_field(mount_point));
        let fields = after.split_whitespace().collect::<Vec<_>>();
        match fields.first().copied() {
            Some("cgroup2") => push_cgroup_root(
                &mut roots.v2,
                &mount_point,
                membership.v2.as_deref().unwrap_or("/"),
            ),
            Some("cgroup") => {
                let options = fields.get(2).copied().unwrap_or_default();
                if options.split(',').any(|option| option == "cpu") {
                    push_cgroup_root(
                        &mut roots.cpu_v1,
                        &mount_point,
                        membership.cpu_v1.as_deref().unwrap_or("/"),
                    );
                }
                if options.split(',').any(|option| option == "cpuset") {
                    push_cgroup_root(
                        &mut roots.cpuset_v1,
                        &mount_point,
                        membership.cpuset_v1.as_deref().unwrap_or("/"),
                    );
                }
            }
            _ => {}
        }
    }
}

fn decode_mount_field(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 3 < bytes.len() {
            let octal = &bytes[index + 1..index + 4];
            if octal.iter().all(|byte| (b'0'..=b'7').contains(byte)) {
                let value = (octal[0] - b'0') * 64 + (octal[1] - b'0') * 8 + octal[2] - b'0';
                decoded.push(value);
                index += 4;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn push_cgroup_root(roots: &mut Vec<PathBuf>, mount_point: &Path, member: &str) {
    let Some(root) = join_cgroup_member(mount_point, member) else {
        return;
    };
    if !roots.contains(&root) {
        roots.push(root);
    }
}

fn join_cgroup_member(mount_point: &Path, member: &str) -> Option<PathBuf> {
    let mut root = mount_point.to_path_buf();
    for component in Path::new(member.trim_start_matches('/')).components() {
        match component {
            Component::Normal(component) => root.push(component),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(root)
}

fn read_v2_cpu_quota(root: &Path) -> Option<CgroupCpuQuota> {
    let source = root.join("cpu.max");
    let contents = fs::read_to_string(&source).ok()?;
    let mut fields = contents.split_whitespace();
    let quota = fields.next();
    let period_us = fields.next().and_then(|value| value.parse::<u64>().ok());
    let (quota_us, unlimited) = match quota {
        Some("max") => (None, Some(true)),
        Some(value) => match value.parse::<u64>() {
            Ok(quota) => (Some(quota), Some(false)),
            Err(_) => (None, None),
        },
        None => (None, None),
    };
    Some(CgroupCpuQuota {
        version: CgroupVersion::V2,
        quota_us,
        period_us,
        unlimited,
        source: source.to_string_lossy().into_owned(),
    })
}

fn read_v1_cpu_quota(root: &Path) -> Option<CgroupCpuQuota> {
    let quota_source = root.join("cpu.cfs_quota_us");
    let period_source = root.join("cpu.cfs_period_us");
    let quota = fs::read_to_string(&quota_source).ok();
    let period = fs::read_to_string(&period_source).ok();
    if quota.is_none() && period.is_none() {
        return None;
    }

    let parsed_quota = quota
        .as_deref()
        .and_then(|value| value.trim().parse::<i64>().ok());
    let (quota_us, unlimited) = match parsed_quota {
        Some(-1) => (None, Some(true)),
        Some(value) if value >= 0 => (Some(value as u64), Some(false)),
        _ => (None, None),
    };
    Some(CgroupCpuQuota {
        version: CgroupVersion::V1,
        quota_us,
        period_us: period
            .as_deref()
            .and_then(|value| value.trim().parse::<u64>().ok()),
        unlimited,
        source: quota_source.to_string_lossy().into_owned(),
    })
}

fn read_v2_cpuset(root: &Path) -> Option<CgroupCpuset> {
    for name in ["cpuset.cpus.effective", "cpuset.cpus"] {
        let source = root.join(name);
        if let Ok(contents) = fs::read_to_string(&source) {
            return Some(CgroupCpuset {
                version: CgroupVersion::V2,
                cpus: contents.trim().to_owned(),
                source: source.to_string_lossy().into_owned(),
            });
        }
    }
    None
}

fn read_v1_cpuset(root: &Path) -> Option<CgroupCpuset> {
    let source = root.join("cpuset.cpus");
    let contents = fs::read_to_string(&source).ok()?;
    Some(CgroupCpuset {
        version: CgroupVersion::V1,
        cpus: contents.trim().to_owned(),
        source: source.to_string_lossy().into_owned(),
    })
}

fn push_cpu_quota_json(json: &mut String, quota: Option<&CgroupCpuQuota>) {
    let Some(quota) = quota else {
        json.push_str("null");
        return;
    };
    json.push_str("{\"version\":");
    push_json_string(json, quota.version.as_str());
    json.push_str(",\"quota_us\":");
    push_optional_number(json, quota.quota_us);
    json.push_str(",\"period_us\":");
    push_optional_number(json, quota.period_us);
    json.push_str(",\"unlimited\":");
    match quota.unlimited {
        Some(value) => json.push_str(if value { "true" } else { "false" }),
        None => json.push_str("null"),
    }
    json.push_str(",\"source\":");
    push_json_string(json, &quota.source);
    json.push('}');
}

fn push_cpuset_json(json: &mut String, cpuset: Option<&CgroupCpuset>) {
    let Some(cpuset) = cpuset else {
        json.push_str("null");
        return;
    };
    json.push_str("{\"version\":");
    push_json_string(json, cpuset.version.as_str());
    json.push_str(",\"cpus\":");
    push_json_string(json, &cpuset.cpus);
    json.push_str(",\"source\":");
    push_json_string(json, &cpuset.source);
    json.push('}');
}

fn push_command_json(json: &mut String, output: &CommandOutput) {
    json.push_str("{\"command\":");
    push_json_string(json, &output.command);
    json.push_str(",\"status\":");
    push_optional_number(json, output.status);
    json.push_str(",\"stdout\":");
    push_json_string(json, &output.stdout);
    json.push_str(",\"stderr\":");
    push_json_string(json, &output.stderr);
    json.push_str(",\"error\":");
    push_optional_string(json, output.error.as_deref());
    json.push('}');
}

fn push_optional_string(json: &mut String, value: Option<&str>) {
    match value {
        Some(value) => push_json_string(json, value),
        None => json.push_str("null"),
    }
}

fn push_optional_number(json: &mut String, value: Option<impl ToString>) {
    match value {
        Some(value) => {
            write!(json, "{}", value.to_string()).unwrap();
        }
        None => json.push_str("null"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!("kfrc-system-test-{}-{id}", process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_cpu_model_and_logical_count() {
        let contents = "processor: 0\nmodel name: Example CPU \n\n\
                        processor : 1\nModel: fallback\nprocessor: invalid\n";
        assert_eq!(
            parse_proc_cpuinfo(contents),
            (Some("Example CPU".to_owned()), Some(2))
        );
        assert_eq!(
            parse_proc_cpuinfo("processor: 0\nModel: ARM processor\n"),
            (Some("ARM processor".to_owned()), Some(1))
        );
        assert_eq!(parse_proc_cpuinfo("unreadable or empty"), (None, None));
    }

    #[test]
    fn parses_memory_with_checked_unit_conversion() {
        assert_eq!(
            parse_proc_meminfo("MemFree: 1 kB\nMemTotal: 2048 kB\n"),
            Some(2_097_152)
        );
        assert_eq!(parse_proc_meminfo("MemTotal: 1 MB\n"), None);
        assert_eq!(parse_proc_meminfo("MemTotal: 1\n"), None);
        assert_eq!(
            parse_proc_meminfo("MemTotal: 18014398509481983 kB\n"),
            Some(18_446_744_073_709_550_592)
        );
        assert_eq!(parse_proc_meminfo("MemTotal: 18014398509481984 kB\n"), None);
        assert_eq!(parse_proc_meminfo("MemAvailable: 1 kB\n"), None);
    }

    #[test]
    fn collects_nested_v2_quota_and_effective_cpuset() {
        let temp = TempDir::new();
        temp.write("proc/self/cgroup", "0::/bench.slice/work\n");
        temp.write("sys/fs/cgroup/bench.slice/work/cpu.max", "250000 100000\n");
        temp.write(
            "sys/fs/cgroup/bench.slice/work/cpuset.cpus.effective",
            "0-3,7\n",
        );
        temp.write("sys/fs/cgroup/bench.slice/work/cpuset.cpus", "0-7\n");

        let info = collect_cgroup(&temp.0.join("proc"), &temp.0.join("sys"));
        let quota = info.cpu_quota.unwrap();
        assert_eq!(quota.version, CgroupVersion::V2);
        assert_eq!(quota.quota_us, Some(250_000));
        assert_eq!(quota.period_us, Some(100_000));
        assert_eq!(quota.unlimited, Some(false));
        let cpuset = info.cpuset.unwrap();
        assert_eq!(cpuset.version, CgroupVersion::V2);
        assert_eq!(cpuset.cpus, "0-3,7");
    }

    #[test]
    fn collects_v1_unlimited_quota_and_cpuset() {
        let temp = TempDir::new();
        temp.write(
            "proc/self/cgroup",
            "2:cpu,cpuacct:/limited\n4:cpuset:/workers\n",
        );
        temp.write("sys/fs/cgroup/cpu,cpuacct/limited/cpu.cfs_quota_us", "-1\n");
        temp.write(
            "sys/fs/cgroup/cpu,cpuacct/limited/cpu.cfs_period_us",
            "100000\n",
        );
        temp.write("sys/fs/cgroup/cpuset/workers/cpuset.cpus", "2,4-5\n");

        let info = collect_cgroup(&temp.0.join("proc"), &temp.0.join("sys"));
        let quota = info.cpu_quota.unwrap();
        assert_eq!(quota.version, CgroupVersion::V1);
        assert_eq!(quota.quota_us, None);
        assert_eq!(quota.period_us, Some(100_000));
        assert_eq!(quota.unlimited, Some(true));
        let cpuset = info.cpuset.unwrap();
        assert_eq!(cpuset.version, CgroupVersion::V1);
        assert_eq!(cpuset.cpus, "2,4-5");
    }

    #[test]
    fn discovers_nonstandard_v2_mount_and_decodes_path() {
        let temp = TempDir::new();
        temp.write("proc/self/cgroup", "0::/job\n");
        let mount_point = temp.0.join("mnt/cgroup space");
        let encoded_mount_point = mount_point.to_string_lossy().replace(' ', "\\040");
        temp.write(
            "proc/self/mountinfo",
            &format!("31 20 0:26 / {encoded_mount_point} rw - cgroup2 cgroup rw\n"),
        );
        temp.write("mnt/cgroup space/job/cpu.max", "max 50000\n");
        temp.write("mnt/cgroup space/job/cpuset.cpus", "1-2\n");

        let info = collect_cgroup(&temp.0.join("proc"), &temp.0.join("sys"));
        let quota = info.cpu_quota.unwrap();
        assert_eq!(quota.version, CgroupVersion::V2);
        assert_eq!(quota.quota_us, None);
        assert_eq!(quota.period_us, Some(50_000));
        assert_eq!(quota.unlimited, Some(true));
        assert_eq!(info.cpuset.unwrap().cpus, "1-2");
        assert_eq!(decode_mount_field("a\\040b\\134c"), "a b\\c");
    }

    #[test]
    fn missing_and_malformed_cgroups_are_explicitly_unknown() {
        let temp = TempDir::new();
        let info = collect_cgroup(&temp.0.join("proc"), &temp.0.join("sys"));
        assert_eq!(info.cpu_quota, None);
        assert_eq!(info.cpuset, None);

        temp.write("proc/self/cgroup", "0::/\n");
        temp.write("sys/fs/cgroup/cpu.max", "not-a-quota not-a-period\n");
        let quota = collect_cgroup(&temp.0.join("proc"), &temp.0.join("sys"))
            .cpu_quota
            .unwrap();
        assert_eq!(quota.quota_us, None);
        assert_eq!(quota.period_us, None);
        assert_eq!(quota.unlimited, None);
    }

    #[test]
    fn rejects_cgroup_member_paths_that_escape_the_mount() {
        assert_eq!(
            join_cgroup_member(Path::new("/sys/fs/cgroup"), "/safe/child"),
            Some(PathBuf::from("/sys/fs/cgroup/safe/child"))
        );
        assert_eq!(
            join_cgroup_member(Path::new("/sys/fs/cgroup"), "/../secret"),
            None
        );
    }

    #[test]
    fn command_spawn_failure_is_data_not_a_collection_failure() {
        let output = run_command(
            "missing --version",
            "/definitely/not/a/kfrc-test-command",
            &["--version"],
        );
        assert_eq!(output.command, "missing --version");
        assert_eq!(output.status, None);
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, "");
        assert!(output.error.is_some());
    }

    #[test]
    fn json_is_complete_escaped_and_uses_null_for_unknown() {
        let info = SystemInfo {
            os: "linux\"test".to_owned(),
            arch: "x86\n64".to_owned(),
            cpu: CpuInfo {
                model: Some("cpu\t\u{1}".to_owned()),
                logical_count: Some(8),
            },
            memory: MemoryInfo { total_bytes: None },
            cgroup: CgroupInfo {
                cpu_quota: Some(CgroupCpuQuota {
                    version: CgroupVersion::V2,
                    quota_us: None,
                    period_us: Some(100_000),
                    unlimited: Some(true),
                    source: "/sys/fs/cgroup/cpu.max".to_owned(),
                }),
                cpuset: None,
            },
            rustc_vv: CommandOutput {
                command: "rustc -Vv".to_owned(),
                status: Some(0),
                stdout: "rustc 1.85\n".to_owned(),
                stderr: String::new(),
                error: None,
            },
            git_version: CommandOutput {
                command: "git --version".to_owned(),
                status: None,
                stdout: String::new(),
                stderr: "bad \\ output".to_owned(),
                error: Some("not found".to_owned()),
            },
        };

        assert_eq!(
            info.to_json(),
            "{\"os\":\"linux\\\"test\",\"arch\":\"x86\\n64\",\
             \"cpu\":{\"model\":\"cpu\\t\\u0001\",\"logical_count\":8},\
             \"memory\":{\"total_bytes\":null},\"cgroup\":{\"cpu_quota\":\
             {\"version\":\"v2\",\"quota_us\":null,\"period_us\":100000,\
             \"unlimited\":true,\"source\":\"/sys/fs/cgroup/cpu.max\"},\
             \"cpuset\":null},\"rustc_vv\":{\"command\":\"rustc -Vv\",\
             \"status\":0,\"stdout\":\"rustc 1.85\\n\",\"stderr\":\"\",\
             \"error\":null},\"git_version\":{\"command\":\"git --version\",\
             \"status\":null,\"stdout\":\"\",\"stderr\":\"bad \\\\ output\",\
             \"error\":\"not found\"}}"
        );
    }

    #[test]
    fn real_collection_always_produces_a_fingerprint() {
        let info = collect_system_info();
        assert_eq!(info.os, std::env::consts::OS);
        assert_eq!(info.arch, std::env::consts::ARCH);
        assert_eq!(info.rustc_vv.command, "rustc -Vv");
        assert_eq!(info.git_version.command, "git --version");
        let json = info.to_json();
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"cgroup\":{\"cpu_quota\":"));
    }
}
