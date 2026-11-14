//! Linux instructions:u over existing threads and their subsequently born tasks.
//! No scaled counts: unsupported PMUs, races and multiplexing are errors.
use std::fs::File;
use std::io;
use std::time::Instant;

#[derive(Debug)]
pub struct RootMeasurement {
    pub tid: u32,
    pub instructions: u64,
    pub enabled_ns: u64,
    pub running_ns: u64,
}

#[derive(Debug)]
pub struct Measurement {
    pub instructions: u64,
    pub elapsed_ns: u128,
    pub roots: Vec<RootMeasurement>,
}

struct Counter {
    tid: u32,
    file: File,
}

pub struct Window {
    counters: Vec<Counter>,
    owner_tid: u32,
    start: Instant,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl Window {
    pub fn start() -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        let before = thread_ids()?;
        // Every initial thread owns one root event; inherit collects future
        // descendants, including terminated LLVM workers, into that root.
        let counters = before
            .iter()
            .map(|&tid| open_counter(tid))
            .collect::<io::Result<Vec<_>>>()?;
        if before != thread_ids()? {
            return Err(io::Error::other(
                "thread set changed while arming inherited counters; coverage is ambiguous",
            ));
        }
        // SAFETY: gettid has no arguments or pointer ownership requirements.
        let owner_tid = unsafe { syscall(186) } as u32;
        let start = Instant::now();
        for counter in &counters {
            perf_ioctl(counter.file.as_raw_fd(), 0x2400)?;
        }
        Ok(Self {
            counters,
            owner_tid,
            start,
        })
    }

    pub fn finish(mut self) -> io::Result<Measurement> {
        use std::io::Read;
        use std::os::fd::AsRawFd;
        for counter in &self.counters {
            perf_ioctl(counter.file.as_raw_fd(), 0x2401)?;
        }
        let elapsed_ns = self.start.elapsed().as_nanos();
        let mut instructions = 0u64;
        let mut roots = Vec::with_capacity(self.counters.len());
        for counter in &mut self.counters {
            let mut bytes = [0u8; 24];
            counter.file.read_exact(&mut bytes)?;
            let read_word =
                |offset| u64::from_ne_bytes(bytes[offset..offset + 8].try_into().unwrap());
            let root = RootMeasurement {
                tid: counter.tid,
                instructions: read_word(0),
                enabled_ns: read_word(8),
                running_ns: read_word(16),
            };
            if root.enabled_ns != root.running_ns || (root.instructions > 0 && root.running_ns == 0)
            {
                return Err(io::Error::other(format!(
                    "tid {} counter unavailable/multiplexed: instructions={}, enabled={}, running={}",
                    root.tid, root.instructions, root.enabled_ns, root.running_ns
                )));
            }
            if root.tid == self.owner_tid && (root.instructions == 0 || root.running_ns == 0) {
                return Err(io::Error::other(
                    "current thread instruction counter did not run",
                ));
            }
            instructions = instructions
                .checked_add(root.instructions)
                .ok_or_else(|| io::Error::other("instruction counter sum overflow"))?;
            roots.push(root);
        }
        if instructions == 0 {
            return Err(io::Error::other(
                "zero instructions are not a valid codegen measurement",
            ));
        }
        Ok(Measurement {
            instructions,
            elapsed_ns,
            roots,
        })
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn thread_ids() -> io::Result<Vec<u32>> {
    let mut tids = std::fs::read_dir("/proc/self/task")?
        .map(|entry| {
            entry?
                .file_name()
                .to_string_lossy()
                .parse::<u32>()
                .map_err(|error| io::Error::other(format!("invalid task id: {error}")))
        })
        .collect::<io::Result<Vec<_>>>()?;
    tids.sort_unstable();
    Ok(tids)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn open_counter(tid: u32) -> io::Result<Counter> {
    use std::os::fd::FromRawFd;
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
        kind: 0,
        size: 128,
        config: 1,
        sample_period: 0,
        sample_type: 0,
        read_format: 3, // TOTAL_TIME_ENABLED | TOTAL_TIME_RUNNING, no GROUP
        flags: 1 | 2 | 4 | 32 | 64, // disabled | inherit | pinned | exclude_kernel | exclude_hv
        reserved: [0; 10],
    };
    // SAFETY: the kernel copies this fully initialized ABI struct. cpu=-1 is
    // per task, group=-1 is standalone, flags=8 closes the event across exec.
    let fd = unsafe {
        syscall(
            298,
            &attr as *const PerfEventAttr,
            tid as i32,
            -1i32,
            -1i32,
            8u64,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: perf_event_open returned a new owned descriptor.
    Ok(Counter {
        tid,
        file: unsafe { File::from_raw_fd(fd as i32) },
    })
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe extern "C" {
    fn syscall(number: i64, ...) -> i64;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn perf_ioctl(fd: i32, request: u64) -> io::Result<()> {
    // SAFETY: enable/disable take integer zero, not a userspace pointer.
    if unsafe { ioctl(fd, request, 0u64) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
impl Window {
    pub fn start() -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "inherited instruction counters require Linux x86-64",
        ))
    }
    pub fn finish(self) -> io::Result<Measurement> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "inherited instruction counters require Linux x86-64",
        ))
    }
}
