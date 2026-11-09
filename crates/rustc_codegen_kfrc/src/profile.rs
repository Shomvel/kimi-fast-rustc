//! Optional exact user-mode instruction windows for the kfrc middle/backend.
//!
//! `KFRC_PROFILE=/absolute/file.jsonl` writes one record per measured window.
//! Unavailable counters, multiplexing and I/O failures never become zero-count
//! successes. These records do not measure LLVM or the excluded MIR converter.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

static OUTPUT: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Run a non-overlapping compiler stage under a hardware instruction counter.
/// The caller owns stage boundaries. Worker-thread work is not covered by this
/// per-thread counter and must use its own window; summing nested windows is invalid.
pub fn measure<T>(stage: &str, symbol: &str, work: impl FnOnce() -> T) -> T {
    if std::env::var_os("KFRC_PROFILE_TOTAL").is_some() {
        return work();
    }
    let Some(path) = OUTPUT.get_or_init(|| std::env::var_os("KFRC_PROFILE").map(PathBuf::from))
    else {
        return work();
    };
    // Open the output before enabling the counter so reporting I/O is excluded.
    let mut output = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("kfrc profile: cannot open {}: {error}", path.display());
            return work();
        }
    };
    let window = Window::start();
    let start = Instant::now();
    let result = work();
    let elapsed_ns = start.elapsed().as_nanos();
    let measurement = window.and_then(Window::finish);
    let base = format!(
        "\"stage\":{},\"symbol\":{},\"pid\":{},\"scope\":\"current_thread_user_instructions\",\"elapsed_ns\":{}",
        json_string(stage),
        json_string(symbol),
        std::process::id(),
        elapsed_ns,
    );
    let line = match measurement {
        Ok(value) => format!(
            "{{{base},\"status\":\"ok\",\"instructions\":{},\"time_enabled_ns\":{},\"time_running_ns\":{}}}\n",
            value.instructions, value.enabled, value.running,
        ),
        Err(error) => format!(
            "{{{base},\"status\":\"unavailable\",\"error\":{}}}\n",
            json_string(&error.to_string()),
        ),
    };
    if let Err(error) = output.write_all(line.as_bytes()) {
        eprintln!("kfrc profile: cannot write {}: {error}", path.display());
    }
    result
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => {
                use std::fmt::Write;
                let _ = write!(escaped, "\\u{:04x}", ch as u32);
            }
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

struct Measurement {
    instructions: u64,
    enabled: u64,
    running: u64,
}

struct Window {
    counter: File,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl Window {
    fn start() -> std::io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};
        // Linux perf_event_attr v7, 128 bytes. Named prefix fields have the
        // kernel ABI offsets; unused trailing options are zeroed.
        #[repr(C)]
        struct PerfEventAttr {
            kind: u32,
            size: u32,
            config: u64,
            sample_period: u64,
            sample_type: u64,
            read_format: u64,
            flags: u64,
            reserved: [u64; 10],
        }
        const _: () = assert!(std::mem::size_of::<PerfEventAttr>() == 128);
        let attr = PerfEventAttr {
            kind: 0, // PERF_TYPE_HARDWARE
            size: 128,
            config: 1, // PERF_COUNT_HW_INSTRUCTIONS
            sample_period: 0,
            sample_type: 0,
            read_format: 3,         // TOTAL_TIME_ENABLED | TOTAL_TIME_RUNNING
            flags: 1 | 4 | 32 | 64, // disabled | pinned | exclude_kernel | exclude_hv
            reserved: [0; 10],
        };
        // SAFETY: attr is initialized with the Linux x86-64 perf ABI layout;
        // the kernel copies it synchronously. pid=0/cpu=-1 selects this thread.
        let fd = unsafe { syscall(298, &attr as *const PerfEventAttr, 0i32, -1i32, -1i32, 8u64) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: successful perf_event_open returns a new owned descriptor.
        let counter = unsafe { File::from_raw_fd(fd as i32) };
        perf_ioctl(counter.as_raw_fd(), 0x2403)?; // PERF_EVENT_IOC_RESET
        perf_ioctl(counter.as_raw_fd(), 0x2400)?; // PERF_EVENT_IOC_ENABLE
        Ok(Self { counter })
    }

    fn finish(mut self) -> std::io::Result<Measurement> {
        use std::os::fd::AsRawFd;
        perf_ioctl(self.counter.as_raw_fd(), 0x2401)?; // PERF_EVENT_IOC_DISABLE
        let mut data = [0u8; 24];
        self.counter.read_exact(&mut data)?;
        let word = |offset| u64::from_ne_bytes(data[offset..offset + 8].try_into().unwrap());
        let result = Measurement {
            instructions: word(0),
            enabled: word(8),
            running: word(16),
        };
        if result.enabled == 0 || result.running != result.enabled {
            return Err(std::io::Error::other(format!(
                "counter did not run continuously: enabled={} running={}",
                result.enabled, result.running,
            )));
        }
        Ok(result)
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe extern "C" {
    fn syscall(number: i64, ...) -> i64;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn perf_ioctl(fd: i32, request: u64) -> std::io::Result<()> {
    // SAFETY: fd is a live perf descriptor; these three requests take integer
    // zero, not a pointer. The descriptor remains owned by the calling Window.
    if unsafe { ioctl(fd, request, 0u64) } < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
impl Window {
    fn start() -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "instruction windows require Linux x86-64",
        ))
    }
    fn finish(self) -> std::io::Result<Measurement> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "instruction windows require Linux x86-64",
        ))
    }
}

// Both backends use exactly the same independently validated Linux counter.
// Keeping one source avoids diverging inheritance or thread enumeration rules.
#[path = "../../rustc_codegen_llvm_profile/src/counter.rs"]
mod inherited_counter;

/// Whole backend entry-to-join window, including collector and MIR conversion.
/// This conservative total is deliberately distinct from the per-stage windows.
pub struct TotalMeasurement {
    output: File,
    path: PathBuf,
    symbol: String,
    counter: Option<std::io::Result<inherited_counter::Window>>,
}

/// Start before collecting or converting codegen items. The caller must finish
/// after `join_codegen` and before linking. With total profiling enabled, the
/// individual stage hooks are disabled to avoid nested hardware counters.
pub fn start_total(symbol: &str) -> Option<TotalMeasurement> {
    let path = PathBuf::from(std::env::var_os("KFRC_PROFILE_TOTAL")?);
    let output = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(output) => output,
        Err(error) => {
            eprintln!(
                "kfrc total profile: cannot open {}: {error}",
                path.display()
            );
            return None;
        }
    };
    Some(TotalMeasurement {
        output,
        path,
        symbol: symbol.into(),
        counter: Some(inherited_counter::Window::start()),
    })
}

impl TotalMeasurement {
    pub fn finish(mut self) {
        let measurement = self
            .counter
            .take()
            .expect("total counter finished once")
            .and_then(inherited_counter::Window::finish);
        let base = format!(
            "\"backend\":\"kfrc\",\"stage\":\"codegen_crate_through_join\",\"symbol\":{},\"pid\":{},\"scope\":\"initial_process_threads_and_descendants_user_instructions\",\"includes_collector_and_mir_conversion\":true,\"nested_stage_windows\":false",
            json_string(&self.symbol),
            std::process::id(),
        );
        let line = match measurement {
            Ok(value) => {
                let roots = value.roots.iter().map(|root| format!(
                    "{{\"tid\":{},\"instructions\":{},\"time_enabled_ns\":{},\"time_running_ns\":{}}}",
                    root.tid, root.instructions, root.enabled_ns, root.running_ns,
                )).collect::<Vec<_>>().join(",");
                format!(
                    "{{{base},\"status\":\"ok\",\"instructions\":{},\"elapsed_ns\":{},\"roots\":[{roots}]}}\n",
                    value.instructions, value.elapsed_ns
                )
            }
            Err(error) => format!(
                "{{{base},\"status\":\"unavailable\",\"error\":{}}}\n",
                json_string(&error.to_string())
            ),
        };
        if let Err(error) = self.output.write_all(line.as_bytes()) {
            eprintln!(
                "kfrc total profile: cannot write {}: {error}",
                self.path.display()
            );
        }
    }
}

impl Drop for TotalMeasurement {
    fn drop(&mut self) {
        if self.counter.take().is_some() {
            let line = format!(
                "{{\"backend\":\"kfrc\",\"stage\":\"codegen_crate_through_join\",\"symbol\":{},\"status\":\"aborted\",\"error\":\"backend did not finish join_codegen\"}}\n",
                json_string(&self.symbol),
            );
            if let Err(error) = self.output.write_all(line.as_bytes()) {
                eprintln!(
                    "kfrc total profile: cannot write {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}
