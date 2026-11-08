//! Batch compilation of `.kfr` sources directly to ELF64 relocatable objects.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use crate::frontend::{self, ParseError};
use crate::native::{self, CompileError};
use crate::native_object::{self, NativeObjectError};
use crate::parallel_native::{self, ParallelCompileError};
use crate::pipeline::{PipelineBuilder, PipelineError};
use crate::pipelined_native::PipelinedNativeError;
use crate::placement::{self, PlacementError};
use crate::scheduler::SchedulerError;
use crate::son::Graph;
use crate::son_optimize;
use crate::x86::MachineFunction;

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
    Pipeline(PipelinedNativeError),
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
            Self::Pipeline(source) => write!(formatter, "pipeline compilation failed: {source}"),
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
            Self::Pipeline(source) => Some(source),
            Self::Object { source, .. } => Some(source),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BatchMode {
    #[default]
    Dag,
    Pipeline,
}

#[derive(Clone)]
struct BatchInput {
    source: PathBuf,
    output: PathBuf,
    symbol: String,
}

/// Compiles one single-function `.kfr` graph per input and writes `<stem>.o` files.
///
/// This retains the DAG compilation mode. See [`compile_batch_with_mode`] for the
/// ordering, parallelism, filename, and output guarantees shared by both modes.
pub fn compile_batch<I, P>(
    inputs: I,
    output_directory: impl AsRef<Path>,
    max_parallelism: usize,
) -> Result<Vec<PathBuf>, NativeBatchError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    compile_batch_with_mode(inputs, output_directory, max_parallelism, BatchMode::Dag)
}

/// Compiles inputs using DAG tasks or a bounded three-stage compilation pipeline.
///
/// Input and returned output paths have the same order. DAG mode uses at most
/// `max_parallelism` workers. Pipeline mode uses that many workers and queue slots
/// for each frontend, optimization, and codegen stage. Object files are written
/// sequentially in input order by the same code in both modes. File stems must be
/// nonempty ASCII alphanumeric, `_`, or `-` strings. Duplicate output names are
/// rejected before any input is read.
pub fn compile_batch_with_mode<I, P>(
    inputs: I,
    output_directory: impl AsRef<Path>,
    max_parallelism: usize,
    mode: BatchMode,
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
    let functions = match mode {
        BatchMode::Dag => compile_dag(&inputs, max_parallelism)?,
        BatchMode::Pipeline => compile_pipeline(&inputs, max_parallelism)?,
    };
    write_objects(&inputs, &functions, output_directory)?;
    Ok(inputs.into_iter().map(|input| input.output).collect())
}

fn compile_dag(
    inputs: &[BatchInput],
    max_parallelism: usize,
) -> Result<Vec<MachineFunction>, NativeBatchError> {
    let mut graphs = Vec::with_capacity(inputs.len());
    for input in inputs {
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

    parallel_native::compile_graphs(graphs, max_parallelism)
        .map_err(|error| compile_error(error, inputs))
}

#[derive(Debug)]
enum StageError {
    Read(io::Error),
    InvalidUtf8(FromUtf8Error),
    Parse(ParseError),
    Placement(PlacementError),
    Compile(CompileError),
}

fn compile_pipeline(
    inputs: &[BatchInput],
    max_parallelism: usize,
) -> Result<Vec<MachineFunction>, NativeBatchError> {
    let pipeline = PipelineBuilder::new()
        .add_stage(max_parallelism, max_parallelism, |input: BatchInput| {
            let bytes = fs::read(&input.source).map_err(StageError::Read)?;
            let source = String::from_utf8(bytes).map_err(StageError::InvalidUtf8)?;
            let graph = frontend::parse(&source).map_err(StageError::Parse)?;
            Ok((input.symbol, graph))
        })
        .add_stage(
            max_parallelism,
            max_parallelism,
            |(name, mut graph): (String, Graph)| {
                son_optimize::optimize(&mut graph);
                placement::place_for_native(&graph)
                    .map(|graph| (name, graph))
                    .map_err(StageError::Placement)
            },
        )
        .add_stage(
            max_parallelism,
            max_parallelism,
            |(name, graph): (String, Graph)| {
                native::compile_graph(&graph, name).map_err(StageError::Compile)
            },
        )
        .output_capacity(max_parallelism)
        .build()
        .map_err(|source| {
            NativeBatchError::Pipeline(PipelinedNativeError::Configuration(source))
        })?;

    pipeline
        .run(inputs.to_vec())
        .map_err(|error| pipeline_error(error, inputs))
}

fn pipeline_error(error: PipelineError<StageError>, inputs: &[BatchInput]) -> NativeBatchError {
    match error {
        PipelineError::Stage { index, source, .. } => match source {
            StageError::Read(source) => NativeBatchError::Read {
                path: inputs[index].source.clone(),
                source,
            },
            StageError::InvalidUtf8(source) => NativeBatchError::InvalidUtf8 {
                path: inputs[index].source.clone(),
                source,
            },
            StageError::Parse(source) => NativeBatchError::Parse {
                path: inputs[index].source.clone(),
                source,
            },
            StageError::Placement(source) => compile_error(
                ParallelCompileError::Placement {
                    index,
                    name: inputs[index].symbol.clone(),
                    source,
                },
                inputs,
            ),
            StageError::Compile(source) => compile_error(
                ParallelCompileError::Compile {
                    index,
                    name: inputs[index].symbol.clone(),
                    source,
                },
                inputs,
            ),
        },
        PipelineError::Panicked { index, message, .. } => compile_error(
            ParallelCompileError::Panicked {
                index,
                name: inputs[index].symbol.clone(),
                message,
            },
            inputs,
        ),
        PipelineError::Spawn { stage, source } => {
            NativeBatchError::Pipeline(PipelinedNativeError::Spawn { stage, source })
        }
        PipelineError::ThreadPanicked { stage, message } => {
            NativeBatchError::Pipeline(PipelinedNativeError::ThreadPanicked { stage, message })
        }
        PipelineError::Disconnected => {
            NativeBatchError::Pipeline(PipelinedNativeError::Disconnected)
        }
    }
}

fn write_objects(
    inputs: &[BatchInput],
    functions: &[MachineFunction],
    output_directory: &Path,
) -> Result<(), NativeBatchError> {
    fs::create_dir_all(output_directory).map_err(|source| {
        NativeBatchError::CreateOutputDirectory {
            path: output_directory.to_path_buf(),
            source,
        }
    })?;
    for (input, function) in inputs.iter().zip(functions) {
        native_object::write_object_file(function, &input.output).map_err(|source| {
            NativeBatchError::Object {
                input: input.source.clone(),
                path: input.output.clone(),
                source,
            }
        })?;
    }
    Ok(())
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
    fn dag_and_pipeline_modes_are_ordered_and_byte_identical() {
        let directory = TestDirectory::new();
        let first = directory.source("first.kfr", "fn main() { return 40 + 2; }");
        let second = directory.source(
            "second.kfr",
            "fn main(x: i64) { let square = x * x; return square + x * x; }",
        );
        let third = directory.source("third.kfr", "fn main() { return 7; }");
        let dag_parallel_directory = directory.path.join("dag-parallel");
        let dag_sequential_directory = directory.path.join("dag-sequential");
        let pipeline_parallel_directory = directory.path.join("pipeline-parallel");
        let pipeline_sequential_directory = directory.path.join("pipeline-sequential");
        let inputs = [&first, &second, &third];

        assert_eq!(BatchMode::default(), BatchMode::Dag);
        let runs = [
            (
                compile_batch(inputs, &dag_parallel_directory, 2).unwrap(),
                &dag_parallel_directory,
            ),
            (
                compile_batch_with_mode(inputs, &dag_sequential_directory, 1, BatchMode::Dag)
                    .unwrap(),
                &dag_sequential_directory,
            ),
            (
                compile_batch_with_mode(
                    inputs,
                    &pipeline_parallel_directory,
                    2,
                    BatchMode::Pipeline,
                )
                .unwrap(),
                &pipeline_parallel_directory,
            ),
            (
                compile_batch_with_mode(
                    inputs,
                    &pipeline_sequential_directory,
                    1,
                    BatchMode::Pipeline,
                )
                .unwrap(),
                &pipeline_sequential_directory,
            ),
        ];

        for (outputs, output_directory) in &runs {
            let expected: Vec<_> = ["first.o", "second.o", "third.o"]
                .iter()
                .map(|name| output_directory.join(name))
                .collect();
            assert_eq!(outputs, &expected);
        }
        for (index, symbol) in ["first", "second", "third"].iter().enumerate() {
            let expected = fs::read(&runs[0].0[index]).unwrap();
            for (outputs, _) in &runs[1..] {
                assert_eq!(expected, fs::read(&outputs[index]).unwrap());
            }
            assert_eq!(&expected[..4], b"\x7fELF");
            assert_eq!(&expected[16..18], &[1, 0]);
            assert_eq!(&expected[18..20], &[62, 0]);
            let symbol = format!("\0{symbol}\0").into_bytes();
            assert!(expected.windows(symbol.len()).any(|bytes| bytes == symbol));
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
    fn read_utf8_parse_and_compile_errors_include_source_paths_in_both_modes() {
        let directory = TestDirectory::new();
        let output = directory.path.join("objects");
        let missing = directory.path.join("missing.kfr");
        let invalid_utf8 = directory.source("utf8.kfr", [0xff, 0xfe]);
        let invalid_source = directory.source("parse.kfr", "fn main() { return missing; }");
        let multiple_functions = directory.source(
            "compile.kfr",
            "fn first() { return 1; } fn second() { return 2; }",
        );

        for mode in [BatchMode::Dag, BatchMode::Pipeline] {
            let error = compile_batch_with_mode([&missing], &output, 2, mode).unwrap_err();
            assert!(matches!(
                &error,
                NativeBatchError::Read { path, .. } if path == &missing
            ));
            assert!(error.to_string().contains(&missing.display().to_string()));

            let error = compile_batch_with_mode([&invalid_utf8], &output, 2, mode).unwrap_err();
            assert!(matches!(
                &error,
                NativeBatchError::InvalidUtf8 { path, .. } if path == &invalid_utf8
            ));
            assert!(
                error
                    .to_string()
                    .contains(&invalid_utf8.display().to_string())
            );

            let error = compile_batch_with_mode([&invalid_source], &output, 2, mode).unwrap_err();
            assert!(matches!(
                &error,
                NativeBatchError::Parse { path, .. } if path == &invalid_source
            ));
            let message = error.to_string();
            assert!(message.contains("parse.kfr:1:20"), "{message}");
            assert!(
                message.contains("unknown variable or constant `missing`"),
                "{message}"
            );

            let error =
                compile_batch_with_mode([&multiple_functions], &output, 2, mode).unwrap_err();
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
    }

    #[test]
    fn output_directory_and_object_errors_include_output_paths_in_both_modes() {
        let directory = TestDirectory::new();
        let input = directory.source("input.kfr", "fn main() { return 42; }");
        let file_output = directory.source("not-a-directory", "occupied");
        let output = directory.path.join("objects");
        let blocked_object = output.join("input.o");
        fs::create_dir_all(&blocked_object).unwrap();

        for mode in [BatchMode::Dag, BatchMode::Pipeline] {
            let error = compile_batch_with_mode([&input], &file_output, 2, mode).unwrap_err();
            assert!(matches!(
                &error,
                NativeBatchError::CreateOutputDirectory { path, .. } if path == &file_output
            ));
            assert!(
                error
                    .to_string()
                    .contains(&file_output.display().to_string())
            );

            let error = compile_batch_with_mode([&input], &output, 2, mode).unwrap_err();
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
}
