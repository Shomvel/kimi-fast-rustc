//! Batch compilation of `.kfr` sources directly to ELF64 relocatable objects.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use crate::frontend::{self, ParseError};
use crate::native_object::{self, NativeObjectError};
use crate::parallel_native::{self, ParallelCompileError};
use crate::scheduler::SchedulerError;

#[derive(Debug)]
pub enum NativeBatchError {
    ZeroParallelism,
    InvalidFileName {
        path: PathBuf,
        reason: &'static str,
    },
    DuplicateOutput {
        output: PathBuf,
        first_input: PathBuf,
        second_input: PathBuf,
    },
    Read {
        path: PathBuf,
        source: io::Error,
    },
    InvalidUtf8 {
        path: PathBuf,
        source: FromUtf8Error,
    },
    Parse {
        path: PathBuf,
        source: ParseError,
    },
    Compile {
        path: PathBuf,
        source: ParallelCompileError,
    },
    Scheduler(SchedulerError),
    CreateOutputDirectory {
        path: PathBuf,
        source: io::Error,
    },
    Object {
        input: PathBuf,
        path: PathBuf,
        source: NativeObjectError,
    },
}

impl fmt::Display for NativeBatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroParallelism => formatter.write_str("parallelism must be greater than zero"),
            Self::InvalidFileName { path, reason } => {
                write!(
                    formatter,
                    "invalid input file name {}: {reason}",
                    path.display()
                )
            }
            Self::DuplicateOutput {
                output,
                first_input,
                second_input,
            } => write!(
                formatter,
                "duplicate output {} for inputs {} and {}",
                output.display(),
                first_input.display(),
                second_input.display()
            ),
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "failed to read input {}: {source}",
                    path.display()
                )
            }
            Self::InvalidUtf8 { path, source } => {
                write!(
                    formatter,
                    "input {} is not valid UTF-8: {source}",
                    path.display()
                )
            }
            Self::Parse { path, source } => write!(formatter, "{}:{source}", path.display()),
            Self::Compile { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Scheduler(source) => write!(formatter, "parallel compilation failed: {source}"),
            Self::CreateOutputDirectory { path, source } => write!(
                formatter,
                "failed to create output directory {}: {source}",
                path.display()
            ),
            Self::Object {
                input,
                path,
                source,
            } => write!(
                formatter,
                "native object emission for input {} (output {}) failed: {source}",
                input.display(),
                path.display()
            ),
        }
    }
}

impl Error for NativeBatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ZeroParallelism | Self::InvalidFileName { .. } | Self::DuplicateOutput { .. } => {
                None
            }
            Self::Read { source, .. } | Self::CreateOutputDirectory { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Compile { source, .. } => Some(source),
            Self::Scheduler(source) => Some(source),
            Self::Object { source, .. } => Some(source),
        }
    }
}

struct BatchInput {
    source: PathBuf,
    output: PathBuf,
    symbol: String,
}

/// Compiles one single-function `.kfr` graph per input and writes `<stem>.o` files.
///
/// Input and returned output paths have the same order. Compilation uses at most
/// `max_parallelism` workers; object files are then written sequentially in input
/// order. File stems must be nonempty ASCII alphanumeric, `_`, or `-` strings.
/// Duplicate output names are rejected before any input is read.
pub fn compile_batch<I, P>(
    inputs: I,
    output_directory: impl AsRef<Path>,
    max_parallelism: usize,
) -> Result<Vec<PathBuf>, NativeBatchError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    if max_parallelism == 0 {
        return Err(NativeBatchError::ZeroParallelism);
    }

    let output_directory = output_directory.as_ref();
    let inputs = prepare_inputs(inputs, output_directory)?;
    let mut graphs = Vec::with_capacity(inputs.len());
    for input in &inputs {
        let bytes = fs::read(&input.source).map_err(|source| NativeBatchError::Read {
            path: input.source.clone(),
            source,
        })?;
        let source = String::from_utf8(bytes).map_err(|source| NativeBatchError::InvalidUtf8 {
            path: input.source.clone(),
            source,
        })?;
        let graph = frontend::parse(&source).map_err(|source| NativeBatchError::Parse {
            path: input.source.clone(),
            source,
        })?;
        graphs.push((input.symbol.clone(), graph));
    }

    let functions = parallel_native::compile_graphs(graphs, max_parallelism)
        .map_err(|error| compile_error(error, &inputs))?;

    fs::create_dir_all(output_directory).map_err(|source| {
        NativeBatchError::CreateOutputDirectory {
            path: output_directory.to_path_buf(),
            source,
        }
    })?;
    for (input, function) in inputs.iter().zip(&functions) {
        native_object::write_object_file(function, &input.output).map_err(|source| {
            NativeBatchError::Object {
                input: input.source.clone(),
                path: input.output.clone(),
                source,
            }
        })?;
    }

    Ok(inputs.into_iter().map(|input| input.output).collect())
}

fn prepare_inputs<I, P>(
    inputs: I,
    output_directory: &Path,
) -> Result<Vec<BatchInput>, NativeBatchError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut prepared: Vec<BatchInput> = Vec::new();
    let mut outputs: HashMap<PathBuf, usize> = HashMap::new();
    for input in inputs {
        let source = input.as_ref().to_path_buf();
        let symbol = safe_file_stem(&source)?;
        let output = output_directory.join(format!("{symbol}.o"));
        if let Some(&first) = outputs.get(&output) {
            return Err(NativeBatchError::DuplicateOutput {
                output,
                first_input: prepared[first].source.clone(),
                second_input: source,
            });
        }
        outputs.insert(output.clone(), prepared.len());
        prepared.push(BatchInput {
            source,
            output,
            symbol,
        });
    }
    Ok(prepared)
}

fn safe_file_stem(path: &Path) -> Result<String, NativeBatchError> {
    let invalid = |reason| NativeBatchError::InvalidFileName {
        path: path.to_path_buf(),
        reason,
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("file name must be valid UTF-8"))?;
    let stem = file_name
        .strip_suffix(".kfr")
        .ok_or_else(|| invalid("expected the .kfr extension"))?;
    if stem.is_empty()
        || !stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(invalid(
            "file stem must contain only ASCII alphanumeric characters, `_`, or `-`",
        ));
    }
    Ok(stem.to_owned())
}

fn compile_error(error: ParallelCompileError, inputs: &[BatchInput]) -> NativeBatchError {
    let index = match &error {
        ParallelCompileError::Scheduler(source) => {
            return NativeBatchError::Scheduler(source.clone());
        }
        ParallelCompileError::Placement { index, .. }
        | ParallelCompileError::Compile { index, .. }
        | ParallelCompileError::Panicked { index, .. } => *index,
    };
    NativeBatchError::Compile {
        path: inputs[index].source.clone(),
        source: error,
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static NEXT_TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "kfrc-native-batch-test-{}-{timestamp:x}-{id:x}",
                process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn source(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
            let path = self.path.join(name);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn compiles_multiple_inputs_in_parallel_in_stable_order() {
        let directory = TestDirectory::new();
        let first = directory.source("first.kfr", "fn main() { return 40 + 2; }");
        let second = directory.source("second.kfr", "fn main(x: i64) { return x * 3; }");
        let third = directory.source("third.kfr", "fn main() { return 7; }");
        let parallel_directory = directory.path.join("parallel");
        let sequential_directory = directory.path.join("sequential");
        let inputs = [&first, &second, &third];

        let parallel = compile_batch(inputs, &parallel_directory, 2).unwrap();
        let sequential = compile_batch(inputs, &sequential_directory, 1).unwrap();

        let names = ["first.o", "second.o", "third.o"];
        let expected_parallel: Vec<_> = names
            .iter()
            .map(|name| parallel_directory.join(name))
            .collect();
        let expected_sequential: Vec<_> = names
            .iter()
            .map(|name| sequential_directory.join(name))
            .collect();
        assert_eq!(parallel, expected_parallel);
        assert_eq!(sequential, expected_sequential);
        for ((parallel_path, sequential_path), symbol) in parallel
            .iter()
            .zip(&sequential)
            .zip(["first", "second", "third"])
        {
            let parallel_object = fs::read(parallel_path).unwrap();
            assert_eq!(parallel_object, fs::read(sequential_path).unwrap());
            assert_eq!(&parallel_object[..4], b"\x7fELF");
            assert_eq!(&parallel_object[16..18], &[1, 0]);
            assert_eq!(&parallel_object[18..20], &[62, 0]);
            let symbol = format!("\0{symbol}\0").into_bytes();
            assert!(
                parallel_object
                    .windows(symbol.len())
                    .any(|bytes| bytes == symbol)
            );
        }
    }

    #[test]
    fn rejects_zero_parallelism_unsafe_names_and_duplicate_outputs_before_io() {
        let directory = TestDirectory::new();
        let missing = directory.path.join("missing.kfr");
        assert!(matches!(
            compile_batch([&missing], directory.path.join("objects"), 0),
            Err(NativeBatchError::ZeroParallelism)
        ));

        for name in [
            "input.rs",
            ".kfr",
            "bad.name.kfr",
            "bad name.kfr",
            "bad\\name.kfr",
        ] {
            let input = directory.path.join(name);
            let error = compile_batch([&input], directory.path.join("objects"), 2).unwrap_err();
            assert!(matches!(
                &error,
                NativeBatchError::InvalidFileName { path, .. } if path == &input
            ));
            assert!(error.to_string().contains(&input.display().to_string()));
        }

        let first = directory.path.join("left").join("shared.kfr");
        let second = directory.path.join("right").join("shared.kfr");
        let output = directory.path.join("objects").join("shared.o");
        let error =
            compile_batch([&first, &second], directory.path.join("objects"), 2).unwrap_err();
        assert!(matches!(
            &error,
            NativeBatchError::DuplicateOutput {
                output: duplicate,
                first_input,
                second_input,
            } if duplicate == &output && first_input == &first && second_input == &second
        ));
        let message = error.to_string();
        assert!(message.contains(&first.display().to_string()), "{message}");
        assert!(message.contains(&second.display().to_string()), "{message}");
        assert!(message.contains(&output.display().to_string()), "{message}");
    }

    #[test]
    fn read_utf8_parse_and_compile_errors_include_source_paths() {
        let directory = TestDirectory::new();
        let output = directory.path.join("objects");
        let missing = directory.path.join("missing.kfr");
        let error = compile_batch([&missing], &output, 2).unwrap_err();
        assert!(matches!(
            &error,
            NativeBatchError::Read { path, .. } if path == &missing
        ));
        assert!(error.to_string().contains(&missing.display().to_string()));

        let invalid_utf8 = directory.source("utf8.kfr", [0xff, 0xfe]);
        let error = compile_batch([&invalid_utf8], &output, 2).unwrap_err();
        assert!(matches!(
            &error,
            NativeBatchError::InvalidUtf8 { path, .. } if path == &invalid_utf8
        ));
        assert!(
            error
                .to_string()
                .contains(&invalid_utf8.display().to_string())
        );

        let invalid_source = directory.source("parse.kfr", "fn main() { return missing; }");
        let error = compile_batch([&invalid_source], &output, 2).unwrap_err();
        assert!(matches!(
            &error,
            NativeBatchError::Parse { path, .. } if path == &invalid_source
        ));
        let message = error.to_string();
        assert!(message.contains("parse.kfr:1:20"), "{message}");
        assert!(message.contains("unknown variable `missing`"), "{message}");

        let multiple_functions = directory.source(
            "compile.kfr",
            "fn first() { return 1; } fn second() { return 2; }",
        );
        let error = compile_batch([&multiple_functions], &output, 2).unwrap_err();
        assert!(matches!(
            &error,
            NativeBatchError::Compile { path, .. } if path == &multiple_functions
        ));
        assert!(
            error
                .to_string()
                .contains(&multiple_functions.display().to_string())
        );
    }

    #[test]
    fn output_directory_and_object_errors_include_output_paths() {
        let directory = TestDirectory::new();
        let input = directory.source("input.kfr", "fn main() { return 42; }");
        let file_output = directory.source("not-a-directory", "occupied");
        let error = compile_batch([&input], &file_output, 2).unwrap_err();
        assert!(matches!(
            &error,
            NativeBatchError::CreateOutputDirectory { path, .. } if path == &file_output
        ));
        assert!(
            error
                .to_string()
                .contains(&file_output.display().to_string())
        );

        let output = directory.path.join("objects");
        let blocked_object = output.join("input.o");
        fs::create_dir_all(&blocked_object).unwrap();
        let error = compile_batch([&input], &output, 2).unwrap_err();
        assert!(matches!(
            &error,
            NativeBatchError::Object { input: source, path, .. }
                if source == &input && path == &blocked_object
        ));
        let message = error.to_string();
        assert!(message.contains(&input.display().to_string()), "{message}");
        assert!(
            message.contains(&blocked_object.display().to_string()),
            "{message}"
        );
    }
}
