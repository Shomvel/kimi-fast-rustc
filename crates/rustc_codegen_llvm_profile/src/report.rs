//! One auditable record per LLVM backend window; no missing-counter zeroes.
use crate::counter::Window;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;

pub struct SessionMeasurement {
    output: File,
    path: PathBuf,
    crate_name: String,
    window: Option<io::Result<Window>>,
    started: Instant,
}

impl SessionMeasurement {
    pub fn start(crate_name: String) -> Option<Self> {
        let path = PathBuf::from(std::env::var_os("KFRC_LLVM_PROFILE")?);
        let output = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("LLVM profile: cannot open {}: {error}", path.display());
                return None;
            }
        };
        let started = Instant::now();
        let window = Some(Window::start());
        Some(Self {
            output,
            path,
            crate_name,
            window,
            started,
        })
    }

    pub fn finish(mut self) {
        let result = self
            .window
            .take()
            .expect("profile window finished once")
            .and_then(Window::finish);
        let base = format!(
            "\"backend\":\"llvm\",\"stage\":\"codegen_crate_through_join\",\"crate\":{},\"pid\":{},\"scope\":\"initial_process_threads_and_descendants_user_instructions\",\"inherit\":true,\"exclude_kernel\":true,\"exclude_hv\":true,\"pinned\":true",
            json_string(&self.crate_name),
            std::process::id(),
        );
        let line = match result {
            Ok(measurement) => {
                let roots = measurement.roots.iter().map(|root| format!(
                    "{{\"tid\":{},\"instructions\":{},\"time_enabled_ns\":{},\"time_running_ns\":{}}}",
                    root.tid, root.instructions, root.enabled_ns, root.running_ns,
                )).collect::<Vec<_>>().join(",");
                format!(
                    "{{{base},\"status\":\"ok\",\"instructions\":{},\"elapsed_ns\":{},\"roots\":[{roots}]}}\n",
                    measurement.instructions, measurement.elapsed_ns
                )
            }
            Err(error) => format!(
                "{{{base},\"status\":\"unavailable\",\"elapsed_ns\":{},\"error\":{}}}\n",
                self.started.elapsed().as_nanos(),
                json_string(&error.to_string())
            ),
        };
        self.write(&line);
    }

    fn write(&mut self, line: &str) {
        if let Err(error) = self.output.write_all(line.as_bytes()) {
            eprintln!(
                "LLVM profile: cannot write {}: {error}",
                self.path.display()
            );
        }
    }
}

impl Drop for SessionMeasurement {
    fn drop(&mut self) {
        if self.window.take().is_some() {
            let line = format!(
                "{{\"backend\":\"llvm\",\"stage\":\"codegen_crate_through_join\",\"crate\":{},\"pid\":{},\"status\":\"aborted\",\"error\":\"LLVM did not complete join_codegen\"}}\n",
                json_string(&self.crate_name),
                std::process::id()
            );
            self.write(&line);
        }
    }
}

fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch.is_control() => {
                use std::fmt::Write;
                let _ = write!(output, "\\u{:04x}", ch as u32);
            }
            ch => output.push(ch),
        }
    }
    output.push('"');
    output
}
