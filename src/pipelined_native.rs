//! Source-to-native compilation as a bounded, cross-input pipeline.
//!
//! Unlike `native_batch`, this module does not schedule graph-internal work.
//! Parsing, optimization/placement, and native compilation can each run multiple
//! workers, with stable input ordering restored at the typed output boundary.

use std::any::Any;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};

use crate::frontend::{self, ParseError};
use crate::native::CompileError;
use crate::native_program::NativeProgram;
use crate::optimizer_pipeline;
use crate::parallel_native::{self, ParallelCompileError};
use crate::pipeline::{PipelineConfigError, PipelineError};
use crate::placement::{self, PlacementError};
use crate::son::Graph;

#[derive(Debug)]
pub enum PipelinedNativeError {
    Configuration(PipelineConfigError),
    Parse {
        index: usize,
        source: ParseError,
    },
    Placement {
        index: usize,
        source: PlacementError,
    },
    Compile {
        index: usize,
        source: CompileError,
    },
    Panicked {
        stage: usize,
        index: usize,
        message: String,
    },
    Spawn {
        stage: Option<usize>,
        source: io::Error,
    },
    ThreadPanicked {
        stage: Option<usize>,
        message: String,
    },
    Disconnected,
}

impl fmt::Display for PipelinedNativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(source) => {
                write!(formatter, "invalid pipeline configuration: {source}")
            }
            Self::Parse { index, source } => {
                write!(formatter, "input {index}: parsing failed: {source}")
            }
            Self::Placement { index, source } => {
                write!(formatter, "input {index}: graph placement failed: {source}")
            }
            Self::Compile { index, source } => {
                write!(
                    formatter,
                    "input {index}: native compilation failed: {source}"
                )
            }
            Self::Panicked {
                stage,
                index,
                message,
            } => write!(
                formatter,
                "pipeline stage {stage} panicked on input {index}: {message}"
            ),
            Self::Spawn { stage, source: _ } => match stage {
                Some(stage) => write!(formatter, "failed to spawn worker for stage {stage}"),
                None => formatter.write_str("failed to spawn pipeline input thread"),
            },
            Self::ThreadPanicked { stage, message } => match stage {
                Some(stage) => write!(
                    formatter,
                    "pipeline stage {stage} worker panicked: {message}"
                ),
                None => write!(formatter, "pipeline input thread panicked: {message}"),
            },
            Self::Disconnected => formatter.write_str(
                "pipeline channel disconnected before every submitted input produced an output",
            ),
        }
    }
}

impl Error for PipelinedNativeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Configuration(source) => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Placement { source, .. } => Some(source),
            Self::Compile { source, .. } => Some(source),
            Self::Spawn { source, .. } => Some(source),
            Self::Panicked { .. } | Self::ThreadPanicked { .. } | Self::Disconnected => None,
        }
    }
}

#[derive(Debug)]
enum StageError {
    Parse(ParseError),
    Placement(PlacementError),
    Compile(CompileError),
    Parallel(ParallelCompileError),
}

/// Compiles named source strings in input order with bounded backpressure.
///
/// This retains one worker per stage. Use [`compile_sources_with_jobs`] to run
/// independent inputs concurrently within each stage. Each result contains all
/// functions declared by the corresponding input.
pub fn compile_sources<I, N, S>(
    inputs: I,
    capacity: usize,
) -> Result<Vec<NativeProgram>, PipelinedNativeError>
where
    I: IntoIterator<Item = (N, S)>,
    I::IntoIter: Send + 'static,
    N: Into<String> + Send + 'static,
    S: Into<String> + Send + 'static,
{
    compile_sources_with_jobs(inputs, capacity, 1)
}

/// Compiles named source strings with at most `jobs` workers per stage.
///
/// `capacity` independently bounds each stage's input queue and the final output
/// queue. Workers preserve each input's index and out-of-order completions are
/// reordered before being returned. Function compilation within an input uses one
/// worker so it does not multiply the per-stage `jobs` limit.
pub fn compile_sources_with_jobs<I, N, S>(
    inputs: I,
    capacity: usize,
    jobs: usize,
) -> Result<Vec<NativeProgram>, PipelinedNativeError>
where
    I: IntoIterator<Item = (N, S)>,
    I::IntoIter: Send + 'static,
    N: Into<String> + Send + 'static,
    S: Into<String> + Send + 'static,
{
    let inputs = inputs
        .into_iter()
        .map(|(name, source)| (name.into(), source.into()));
    run_three_stages(
        inputs,
        capacity,
        capacity,
        [jobs; 3],
        |(name, source): (String, String)| {
            frontend::parse(&source)
                .map(|graph| (name, graph))
                .map_err(StageError::Parse)
        },
        |(name, mut graph): (String, Graph)| {
            optimizer_pipeline::optimize(&mut graph);
            placement::place_for_native(&graph)
                .map(|graph| (name, graph))
                .map_err(StageError::Placement)
        },
        |(name, graph): (String, Graph)| {
            compile_program(name, &graph).map_err(parallel_stage_error)
        },
    )
    .map_err(map_pipeline_error)
}

pub(crate) fn compile_program(
    name: String,
    graph: &Graph,
) -> Result<NativeProgram, ParallelCompileError> {
    let single_function = graph.functions.len() <= 1;
    let mut program = parallel_native::compile_graph(graph, 1)?;
    if single_function {
        let function = &mut program.functions[0];
        let original_name = std::mem::replace(&mut function.name, name.clone());
        for symbol in &mut function.symbols {
            if symbol == &original_name {
                symbol.clone_from(&name);
            }
        }
    }
    Ok(program)
}

fn parallel_stage_error(error: ParallelCompileError) -> StageError {
    match error {
        ParallelCompileError::Placement { source, .. } => StageError::Placement(source),
        ParallelCompileError::Compile { source, .. } => StageError::Compile(source),
        error => StageError::Parallel(error),
    }
}

#[derive(Debug)]
pub(crate) enum RunError<E> {
    Configuration(PipelineConfigError),
    Pipeline(PipelineError<E>),
}

pub(crate) fn run_three_stages<In, Parsed, Placed, Out, E, I, Parse, Optimize, Compile>(
    inputs: I,
    capacity: usize,
    output_capacity: usize,
    parallelism: [usize; 3],
    parse: Parse,
    optimize: Optimize,
    compile: Compile,
) -> Result<Vec<Out>, RunError<E>>
where
    In: Send + 'static,
    Parsed: Send + 'static,
    Placed: Send + 'static,
    Out: Send + 'static,
    E: Send + 'static,
    I: IntoIterator<Item = In>,
    I::IntoIter: Send + 'static,
    Parse: Fn(In) -> Result<Parsed, E> + Send + Sync + 'static,
    Optimize: Fn(Parsed) -> Result<Placed, E> + Send + Sync + 'static,
    Compile: Fn(Placed) -> Result<Out, E> + Send + Sync + 'static,
{
    for (stage, &workers) in parallelism.iter().enumerate() {
        if workers == 0 {
            return Err(RunError::Configuration(
                PipelineConfigError::ZeroParallelism { stage },
            ));
        }
    }
    if capacity == 0 {
        return Err(RunError::Configuration(PipelineConfigError::ZeroCapacity {
            stage: 0,
        }));
    }
    if output_capacity == 0 {
        return Err(RunError::Configuration(
            PipelineConfigError::ZeroOutputCapacity,
        ));
    }

    let (input_sender, input_receiver) = mpsc::sync_channel(capacity);
    let (parsed_sender, parsed_receiver) = mpsc::sync_channel(capacity);
    let (placed_sender, placed_receiver) = mpsc::sync_channel(capacity);
    let (output_sender, output_receiver) = mpsc::sync_channel(output_capacity);
    let cancelled = Arc::new(AtomicBool::new(false));
    let failure = Arc::new(Mutex::new(None));
    let submitted = Arc::new(AtomicUsize::new(0));

    let input_iterator = inputs.into_iter();
    let output_len = input_iterator.size_hint().0;
    let input_cancelled = Arc::clone(&cancelled);
    let input_submitted = Arc::clone(&submitted);
    let input_handle = thread::Builder::new()
        .name("pipeline-input".to_owned())
        .spawn(move || {
            let mut inputs = input_iterator;
            let mut index = 0;
            while !input_cancelled.load(Ordering::Acquire) {
                let Some(value) = inputs.next() else {
                    break;
                };
                if input_cancelled.load(Ordering::Acquire) {
                    break;
                }
                if input_sender.send(Task { index, value }).is_err() {
                    break;
                }
                input_submitted.fetch_add(1, Ordering::Release);
                index += 1;
            }
        });
    let input_handle = match input_handle {
        Ok(handle) => handle,
        Err(source) => {
            return Err(RunError::Pipeline(PipelineError::Spawn {
                stage: None,
                source,
            }));
        }
    };
    let mut threads = vec![WorkerThread {
        stage: None,
        handle: input_handle,
    }];

    match spawn_workers(
        input_receiver,
        parsed_sender,
        parsed_receiver,
        placed_sender,
        placed_receiver,
        output_sender,
        parallelism,
        parse,
        optimize,
        compile,
        &cancelled,
        &failure,
    ) {
        Ok(mut workers) => threads.append(&mut workers),
        Err((mut workers, error)) => {
            threads.append(&mut workers);
            record_failure(&failure, &cancelled, error);
        }
    }

    let mut outputs = Vec::with_capacity(output_len);
    let mut pending = BTreeMap::new();
    let mut invalid_output = false;
    while !cancelled.load(Ordering::Acquire) {
        match output_receiver.recv() {
            Ok(task) => {
                if task.index < outputs.len() || pending.insert(task.index, task.value).is_some() {
                    invalid_output = true;
                    break;
                }
                while let Some(value) = pending.remove(&outputs.len()) {
                    outputs.push(value);
                }
            }
            Err(_) => break,
        }
    }
    drop(output_receiver);
    cancelled.store(true, Ordering::Release);
    if invalid_output {
        record_failure(&failure, &cancelled, PipelineError::Disconnected);
    }

    for worker in threads {
        if let Err(payload) = worker.handle.join() {
            record_failure(
                &failure,
                &cancelled,
                PipelineError::ThreadPanicked {
                    stage: worker.stage,
                    message: panic_message(payload),
                },
            );
        }
    }

    if let Some(error) = lock_unpoisoned(&failure).take() {
        return Err(RunError::Pipeline(error));
    }
    if outputs.len() != submitted.load(Ordering::Acquire) || !pending.is_empty() {
        return Err(RunError::Pipeline(PipelineError::Disconnected));
    }
    Ok(outputs)
}

struct Task<T> {
    index: usize,
    value: T,
}

struct WorkerThread {
    stage: Option<usize>,
    handle: JoinHandle<()>,
}

#[allow(clippy::too_many_arguments)]
fn spawn_workers<In, Parsed, Placed, Out, E, Parse, Optimize, Compile>(
    input_receiver: mpsc::Receiver<Task<In>>,
    parsed_sender: mpsc::SyncSender<Task<Parsed>>,
    parsed_receiver: mpsc::Receiver<Task<Parsed>>,
    placed_sender: mpsc::SyncSender<Task<Placed>>,
    placed_receiver: mpsc::Receiver<Task<Placed>>,
    output_sender: mpsc::SyncSender<Task<Out>>,
    parallelism: [usize; 3],
    parse: Parse,
    optimize: Optimize,
    compile: Compile,
    cancelled: &Arc<AtomicBool>,
    failure: &Arc<Mutex<Option<PipelineError<E>>>>,
) -> Result<Vec<WorkerThread>, (Vec<WorkerThread>, PipelineError<E>)>
where
    In: Send + 'static,
    Parsed: Send + 'static,
    Placed: Send + 'static,
    Out: Send + 'static,
    E: Send + 'static,
    Parse: Fn(In) -> Result<Parsed, E> + Send + Sync + 'static,
    Optimize: Fn(Parsed) -> Result<Placed, E> + Send + Sync + 'static,
    Compile: Fn(Placed) -> Result<Out, E> + Send + Sync + 'static,
{
    let mut workers = Vec::with_capacity(parallelism.iter().sum());
    match spawn_stage(
        0,
        parallelism[0],
        input_receiver,
        parsed_sender,
        parse,
        cancelled,
        failure,
    ) {
        Ok(mut stage_workers) => workers.append(&mut stage_workers),
        Err((mut stage_workers, error)) => {
            workers.append(&mut stage_workers);
            return Err((workers, error));
        }
    }
    match spawn_stage(
        1,
        parallelism[1],
        parsed_receiver,
        placed_sender,
        optimize,
        cancelled,
        failure,
    ) {
        Ok(mut stage_workers) => workers.append(&mut stage_workers),
        Err((mut stage_workers, error)) => {
            workers.append(&mut stage_workers);
            return Err((workers, error));
        }
    }
    match spawn_stage(
        2,
        parallelism[2],
        placed_receiver,
        output_sender,
        compile,
        cancelled,
        failure,
    ) {
        Ok(mut stage_workers) => workers.append(&mut stage_workers),
        Err((mut stage_workers, error)) => {
            workers.append(&mut stage_workers);
            return Err((workers, error));
        }
    }
    Ok(workers)
}

fn spawn_stage<In, Out, E, F>(
    stage: usize,
    parallelism: usize,
    input: mpsc::Receiver<Task<In>>,
    output: mpsc::SyncSender<Task<Out>>,
    function: F,
    cancelled: &Arc<AtomicBool>,
    failure: &Arc<Mutex<Option<PipelineError<E>>>>,
) -> Result<Vec<WorkerThread>, (Vec<WorkerThread>, PipelineError<E>)>
where
    In: Send + 'static,
    Out: Send + 'static,
    E: Send + 'static,
    F: Fn(In) -> Result<Out, E> + Send + Sync + 'static,
{
    let input = Arc::new(Mutex::new(input));
    let function = Arc::new(function);
    let mut workers = Vec::with_capacity(parallelism);
    for worker_index in 0..parallelism {
        let input = Arc::clone(&input);
        let output = output.clone();
        let function = Arc::clone(&function);
        let worker_cancelled = Arc::clone(cancelled);
        let worker_failure = Arc::clone(failure);
        let name = format!("pipeline-stage-{stage}-worker-{worker_index}");
        match thread::Builder::new().name(name).spawn(move || {
            run_stage(
                stage,
                input,
                output,
                function,
                worker_cancelled,
                worker_failure,
            );
        }) {
            Ok(handle) => workers.push(WorkerThread {
                stage: Some(stage),
                handle,
            }),
            Err(source) => {
                return Err((
                    workers,
                    PipelineError::Spawn {
                        stage: Some(stage),
                        source,
                    },
                ));
            }
        }
    }
    Ok(workers)
}

fn run_stage<In, Out, E, F>(
    stage: usize,
    input: Arc<Mutex<mpsc::Receiver<Task<In>>>>,
    output: mpsc::SyncSender<Task<Out>>,
    function: Arc<F>,
    cancelled: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<PipelineError<E>>>>,
) where
    F: Fn(In) -> Result<Out, E>,
{
    while !cancelled.load(Ordering::Acquire) {
        let task = {
            let input = lock_unpoisoned(&input);
            input.recv()
        };
        let Ok(task) = task else {
            break;
        };
        if cancelled.load(Ordering::Acquire) {
            break;
        }

        let Task { index, value } = task;
        match panic::catch_unwind(AssertUnwindSafe(|| function(value))) {
            Ok(Ok(value)) => {
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                if output.send(Task { index, value }).is_err() {
                    break;
                }
            }
            Ok(Err(source)) => {
                record_failure(
                    &failure,
                    &cancelled,
                    PipelineError::Stage {
                        stage,
                        index,
                        source,
                    },
                );
                break;
            }
            Err(payload) => {
                record_failure(
                    &failure,
                    &cancelled,
                    PipelineError::Panicked {
                        stage,
                        index,
                        message: panic_message(payload),
                    },
                );
                break;
            }
        }
    }
}

fn record_failure<E>(
    failure: &Mutex<Option<PipelineError<E>>>,
    cancelled: &AtomicBool,
    error: PipelineError<E>,
) {
    {
        let mut failure = lock_unpoisoned(failure);
        if failure.is_none() {
            *failure = Some(error);
        }
    }
    cancelled.store(true, Ordering::Release);
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn panic_message(payload: Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

fn map_pipeline_error(error: RunError<StageError>) -> PipelinedNativeError {
    let error = match error {
        RunError::Configuration(source) => return PipelinedNativeError::Configuration(source),
        RunError::Pipeline(error) => error,
    };
    match error {
        PipelineError::Stage { index, source, .. } => match source {
            StageError::Parse(source) => PipelinedNativeError::Parse { index, source },
            StageError::Placement(source) => PipelinedNativeError::Placement { index, source },
            StageError::Compile(source) => PipelinedNativeError::Compile { index, source },
            StageError::Parallel(source) => match source {
                ParallelCompileError::Placement { source, .. } => {
                    PipelinedNativeError::Placement { index, source }
                }
                ParallelCompileError::Compile { source, .. } => {
                    PipelinedNativeError::Compile { index, source }
                }
                ParallelCompileError::Panicked { message, .. } => PipelinedNativeError::Panicked {
                    stage: 2,
                    index,
                    message,
                },
                ParallelCompileError::Scheduler(_) => {
                    unreachable!("function compilation always uses one worker")
                }
            },
        },
        PipelineError::Panicked {
            stage,
            index,
            message,
        } => PipelinedNativeError::Panicked {
            stage,
            index,
            message,
        },
        PipelineError::Spawn { stage, source } => PipelinedNativeError::Spawn { stage, source },
        PipelineError::ThreadPanicked { stage, message } => {
            PipelinedNativeError::ThreadPanicked { stage, message }
        }
        PipelineError::Disconnected => PipelinedNativeError::Disconnected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native;
    use std::ops::Range;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Condvar, Mutex, mpsc};
    use std::thread;
    use std::time::Duration;

    fn source(value: i64) -> String {
        format!("fn main() {{ return {value}; }}")
    }

    #[test]
    fn real_stages_compile_in_stable_input_order() {
        let inputs = vec![
            (
                "first".to_owned(),
                "fn main() { return 40 + 2; }".to_owned(),
            ),
            (
                "second".to_owned(),
                "fn main(x: i64) { return x * 3; }".to_owned(),
            ),
            ("third".to_owned(), source(7)),
        ];
        let expected: Vec<Vec<_>> = inputs
            .iter()
            .map(|(name, source)| {
                let mut graph = frontend::parse(source).unwrap();
                optimizer_pipeline::optimize(&mut graph);
                let graph = placement::place_graph(&graph).unwrap();
                vec![native::compile_graph(&graph, name.clone()).unwrap()]
            })
            .collect();

        let programs = compile_sources(inputs, 2).unwrap();

        assert_eq!(programs.len(), expected.len());
        for (program, expected) in programs.iter().zip(expected) {
            assert_eq!(program.functions, expected);
        }
        let names: Vec<_> = programs
            .iter()
            .map(|program| program.functions[0].name.as_str())
            .collect();
        assert_eq!(names, ["first", "second", "third"]);
    }

    #[test]
    fn different_inputs_can_occupy_all_three_stages_simultaneously() {
        let rendezvous = Arc::new(Barrier::new(3));
        let parse_rendezvous = Arc::clone(&rendezvous);
        let optimize_rendezvous = Arc::clone(&rendezvous);
        let compile_rendezvous = Arc::clone(&rendezvous);

        let outputs = run_three_stages(
            0..5,
            1,
            1,
            [1; 3],
            move |input| {
                if input == 2 {
                    parse_rendezvous.wait();
                }
                Ok::<_, &'static str>(input + 1)
            },
            move |input| {
                if input == 2 {
                    optimize_rendezvous.wait();
                }
                Ok(input * 2)
            },
            move |input| {
                if input == 2 {
                    compile_rendezvous.wait();
                }
                Ok(input + 3)
            },
        )
        .unwrap();

        assert_eq!(
            outputs,
            (0..5).map(|input| (input + 1) * 2 + 3).collect::<Vec<_>>()
        );
    }

    struct ObservedInputs {
        range: Range<usize>,
        next: mpsc::Sender<usize>,
    }

    impl Iterator for ObservedInputs {
        type Item = usize;

        fn next(&mut self) -> Option<Self::Item> {
            let next = self.range.next()?;
            self.next.send(next).unwrap();
            Some(next)
        }
    }

    #[test]
    fn bounded_first_stage_backpressures_the_source_iterator() {
        let (next_tx, next_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let stage_gate = Arc::clone(&gate);
        let inputs = ObservedInputs {
            range: 0..64,
            next: next_tx,
        };

        let runner = thread::spawn(move || {
            run_three_stages(
                inputs,
                1,
                1,
                [1; 3],
                move |input| {
                    started_tx.send(input).unwrap();
                    let (lock, condition) = &*stage_gate;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = condition.wait(released).unwrap();
                    }
                    Ok::<_, &'static str>(input)
                },
                |input| Ok(input + 1),
                |input| Ok(input * 2),
            )
        });

        assert_eq!(started_rx.recv_timeout(Duration::from_secs(1)), Ok(0));
        assert_eq!(next_rx.recv_timeout(Duration::from_secs(1)), Ok(0));
        assert_eq!(next_rx.recv_timeout(Duration::from_secs(1)), Ok(1));
        assert_eq!(next_rx.recv_timeout(Duration::from_secs(1)), Ok(2));
        let blocked = next_rx.recv_timeout(Duration::from_millis(75));

        let (lock, condition) = &*gate;
        *lock.lock().unwrap() = true;
        condition.notify_all();
        let outputs = runner.join().unwrap().unwrap();

        assert!(matches!(blocked, Err(mpsc::RecvTimeoutError::Timeout)));
        assert_eq!(
            outputs,
            (0..64).map(|input| (input + 1) * 2).collect::<Vec<_>>()
        );
    }

    #[test]
    fn parse_errors_carry_the_input_index_and_multiple_functions_compile() {
        let parse_error = compile_sources(
            vec![
                ("first", source(1)),
                ("broken", "fn main() { return missing; }".to_owned()),
                ("last", source(3)),
            ],
            1,
        )
        .unwrap_err();
        assert!(matches!(
            &parse_error,
            PipelinedNativeError::Parse { index: 1, .. }
        ));
        assert!(parse_error.to_string().contains("input 1"));

        let programs = compile_sources(
            vec![
                ("first", source(1)),
                (
                    "multiple",
                    "fn first() { return 1; } fn second() { return 2; }".to_owned(),
                ),
                ("last", source(3)),
            ],
            1,
        )
        .unwrap();
        let names: Vec<_> = programs
            .iter()
            .flat_map(|program| {
                program
                    .functions
                    .iter()
                    .map(|function| function.name.as_str())
            })
            .collect();
        assert_eq!(names, ["first", "first", "second", "last"]);
    }

    #[test]
    fn native_errors_carry_the_input_index() {
        let result = run_three_stages(0..4, 1, 1, [4; 3], Ok::<_, StageError>, Ok, |input| {
            if input == 2 {
                Err(StageError::Compile(CompileError::MissingReturn))
            } else {
                Ok(input)
            }
        });
        let error = result.map_err(map_pipeline_error).unwrap_err();

        assert!(matches!(
            &error,
            PipelinedNativeError::Compile {
                index: 2,
                source: CompileError::MissingReturn,
            }
        ));
        assert!(error.to_string().contains("input 2"));
    }

    #[test]
    fn errors_stop_the_bounded_pipeline_without_consuming_every_input() {
        const INPUTS: usize = 256;
        let consumed = Arc::new(AtomicUsize::new(0));
        let iterator_consumed = Arc::clone(&consumed);
        let inputs = (0..INPUTS).inspect(move |_| {
            iterator_consumed.fetch_add(1, Ordering::Relaxed);
        });

        let result = run_three_stages(
            inputs,
            1,
            1,
            [4; 3],
            Ok::<_, &'static str>,
            |input| {
                if input == 0 {
                    Err("placement failed")
                } else {
                    Ok(input)
                }
            },
            Ok,
        );

        assert!(matches!(
            result,
            Err(RunError::Pipeline(PipelineError::Stage {
                stage: 1,
                index: 0,
                source: "placement failed",
            }))
        ));
        assert!(consumed.load(Ordering::Relaxed) < INPUTS);
    }

    #[test]
    fn panics_carry_the_stage_and_input_index() {
        let result = run_three_stages(
            0..8,
            1,
            1,
            [4; 3],
            Ok::<_, &'static str>,
            |input| {
                if input == 3 {
                    panic!("optimizer exploded");
                }
                Ok(input)
            },
            Ok,
        );

        assert!(matches!(
            result,
            Err(RunError::Pipeline(PipelineError::Panicked {
                stage: 1,
                index: 3,
                message,
            })) if message == "optimizer exploded"
        ));
    }

    struct ActiveGuard(Arc<AtomicUsize>);

    impl Drop for ActiveGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn enter_stage(active: &Arc<AtomicUsize>, peak: &AtomicUsize) -> ActiveGuard {
        let active_now = active.fetch_add(1, Ordering::Relaxed) + 1;
        peak.fetch_max(active_now, Ordering::Relaxed);
        ActiveGuard(Arc::clone(active))
    }

    #[test]
    fn parallel_workers_reorder_out_of_order_completion() {
        const WORKERS: usize = 4;
        let rendezvous = Arc::new(Barrier::new(WORKERS));
        let parse_rendezvous = Arc::clone(&rendezvous);
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let parse_gate = Arc::clone(&gate);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let parse_active = Arc::clone(&active);
        let parse_peak = Arc::clone(&peak);
        let (completed_tx, completed_rx) = mpsc::channel();

        let runner = thread::spawn(move || {
            run_three_stages(
                0..32,
                2,
                2,
                [WORKERS; 3],
                move |input| {
                    let _guard = enter_stage(&parse_active, &parse_peak);
                    if input < WORKERS {
                        parse_rendezvous.wait();
                    }
                    if input == 0 {
                        let (lock, condition) = &*parse_gate;
                        let mut released = lock.lock().unwrap();
                        while !*released {
                            released = condition.wait(released).unwrap();
                        }
                    }
                    Ok::<_, &'static str>(input)
                },
                |input| Ok(input + 1),
                move |input| {
                    completed_tx.send(input).unwrap();
                    Ok(input * 2)
                },
            )
        });

        for _ in 1..WORKERS {
            let completed = completed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_ne!(completed, 1, "input 0 completed before its gate opened");
        }
        let (lock, condition) = &*gate;
        *lock.lock().unwrap() = true;
        condition.notify_all();
        let outputs = runner.join().unwrap().unwrap();

        assert_eq!(peak.load(Ordering::Relaxed), WORKERS);
        assert_eq!(
            outputs,
            (0..32).map(|input| (input + 1) * 2).collect::<Vec<_>>()
        );
    }

    #[test]
    fn bounded_parallel_first_stage_backpressures_the_source_iterator() {
        const WORKERS: usize = 3;
        let (next_tx, next_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let rendezvous = Arc::new(Barrier::new(WORKERS));
        let stage_rendezvous = Arc::clone(&rendezvous);
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let stage_gate = Arc::clone(&gate);
        let inputs = ObservedInputs {
            range: 0..64,
            next: next_tx,
        };

        let runner = thread::spawn(move || {
            run_three_stages(
                inputs,
                1,
                1,
                [WORKERS; 3],
                move |input| {
                    started_tx.send(input).unwrap();
                    if input < WORKERS {
                        stage_rendezvous.wait();
                    }
                    let (lock, condition) = &*stage_gate;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = condition.wait(released).unwrap();
                    }
                    Ok::<_, &'static str>(input)
                },
                |input| Ok(input + 1),
                |input| Ok(input * 2),
            )
        });

        let mut started = Vec::new();
        for _ in 0..WORKERS {
            started.push(started_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        }
        started.sort_unstable();
        assert_eq!(started, [0, 1, 2]);
        for expected in 0..=WORKERS + 1 {
            assert_eq!(
                next_rx.recv_timeout(Duration::from_secs(1)),
                Ok(expected),
                "source read ahead beyond the active workers and one queued input"
            );
        }
        let blocked = next_rx.recv_timeout(Duration::from_millis(75));

        let (lock, condition) = &*gate;
        *lock.lock().unwrap() = true;
        condition.notify_all();
        let outputs = runner.join().unwrap().unwrap();

        assert!(matches!(blocked, Err(mpsc::RecvTimeoutError::Timeout)));
        assert_eq!(
            outputs,
            (0..64).map(|input| (input + 1) * 2).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rejects_zero_parallelism_and_queue_capacities() {
        let zero_workers = run_three_stages(0..1, 1, 1, [0; 3], Ok::<_, &'static str>, Ok, Ok);
        assert!(matches!(
            zero_workers,
            Err(RunError::Configuration(
                PipelineConfigError::ZeroParallelism { stage: 0 }
            ))
        ));

        let zero_stage = run_three_stages(0..1, 0, 1, [1; 3], Ok::<_, &'static str>, Ok, Ok);
        assert!(matches!(
            zero_stage,
            Err(RunError::Configuration(PipelineConfigError::ZeroCapacity {
                stage: 0
            }))
        ));

        let zero_output = run_three_stages(0..1, 1, 0, [1; 3], Ok::<_, &'static str>, Ok, Ok);
        assert!(matches!(
            zero_output,
            Err(RunError::Configuration(
                PipelineConfigError::ZeroOutputCapacity
            ))
        ));
    }

    #[test]
    #[ignore = "throughput benchmark; run explicitly"]
    fn executor_throughput_comparison() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        const INPUTS: usize = 250_000;
        const ROUNDS: usize = 9;

        fn run(inputs: std::ops::Range<usize>, jobs: usize) -> Vec<usize> {
            run_three_stages(
                inputs,
                4,
                4,
                [jobs; 3],
                |input| Ok::<_, &'static str>(input + 1),
                |input| Ok(input * 2),
                |input| Ok(input + 3),
            )
            .unwrap()
        }

        fn measure(inputs: std::ops::Range<usize>, jobs: usize) -> (Duration, Vec<usize>) {
            let start = Instant::now();
            let outputs = run(inputs, jobs);
            let elapsed = start.elapsed();
            black_box(&outputs);
            (elapsed, outputs)
        }

        measure(0..INPUTS, 1);
        measure(0..INPUTS, 4);
        let mut baseline_times = Vec::new();
        let mut candidate_times = Vec::new();
        for round in 0..ROUNDS {
            let (baseline, candidate) = if round % 2 == 0 {
                let baseline = measure(0..INPUTS, 1);
                let candidate = measure(0..INPUTS, 4);
                (baseline, candidate)
            } else {
                let candidate = measure(0..INPUTS, 4);
                let baseline = measure(0..INPUTS, 1);
                (baseline, candidate)
            };
            assert_eq!(baseline.1, candidate.1);
            println!(
                "round={} baseline_ms={:.3} candidate_ms={:.3}",
                round + 1,
                baseline.0.as_secs_f64() * 1000.0,
                candidate.0.as_secs_f64() * 1000.0,
            );
            baseline_times.push(baseline.0);
            candidate_times.push(candidate.0);
        }
        baseline_times.sort_unstable();
        candidate_times.sort_unstable();
        let baseline_median = baseline_times[ROUNDS / 2];
        let candidate_median = candidate_times[ROUNDS / 2];
        println!(
            "SUMMARY inputs={INPUTS} rounds={ROUNDS} baseline_median_ms={:.3} candidate_median_ms={:.3} median_speedup={:.3}x",
            baseline_median.as_secs_f64() * 1000.0,
            candidate_median.as_secs_f64() * 1000.0,
            baseline_median.as_secs_f64() / candidate_median.as_secs_f64(),
        );
    }
}
