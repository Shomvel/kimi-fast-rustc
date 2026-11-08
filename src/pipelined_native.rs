//! Source-to-native compilation as a bounded, cross-input pipeline.
//!
//! Unlike `native_batch`, this module does not schedule graph-internal work.
//! Parsing, optimization/placement, and native compilation each have one worker;
//! parallelism comes from different inputs occupying different stages.

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};

use crate::frontend::{self, ParseError};
use crate::native::{self, CompileError};
use crate::optimizer_pipeline;
use crate::pipeline::{PipelineConfigError, PipelineError};
use crate::placement::{self, PlacementError};
use crate::son::Graph;
use crate::x86::MachineFunction;

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
}

/// Compiles named source strings in input order with bounded backpressure.
///
/// `capacity` independently bounds each stage's input queue and the final output
/// queue. Every stage has exactly one worker, so no instruction-level or DAG
/// parallelism is introduced; separate inputs can still occupy all three stages.
pub fn compile_sources<I, N, S>(
    inputs: I,
    capacity: usize,
) -> Result<Vec<MachineFunction>, PipelinedNativeError>
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
            native::compile_graph(&graph, name).map_err(StageError::Compile)
        },
    )
    .map_err(map_pipeline_error)
}

#[derive(Debug)]
enum RunError<E> {
    Configuration(PipelineConfigError),
    Pipeline(PipelineError<E>),
}

fn run_three_stages<In, Parsed, Placed, Out, E, I, Parse, Optimize, Compile>(
    inputs: I,
    capacity: usize,
    output_capacity: usize,
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
    let mut invalid_output = false;
    while !cancelled.load(Ordering::Acquire) {
        match output_receiver.recv() {
            Ok(task) => {
                // One worker per stage makes every channel, including the output, FIFO.
                if task.index != outputs.len() {
                    invalid_output = true;
                    break;
                }
                outputs.push(task.value);
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
    if outputs.len() != submitted.load(Ordering::Acquire) {
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
    Parse: Fn(In) -> Result<Parsed, E> + Send + 'static,
    Optimize: Fn(Parsed) -> Result<Placed, E> + Send + 'static,
    Compile: Fn(Placed) -> Result<Out, E> + Send + 'static,
{
    let mut workers = Vec::with_capacity(3);
    match spawn_stage(0, input_receiver, parsed_sender, parse, cancelled, failure) {
        Ok(worker) => workers.push(worker),
        Err(error) => return Err((workers, error)),
    }
    match spawn_stage(
        1,
        parsed_receiver,
        placed_sender,
        optimize,
        cancelled,
        failure,
    ) {
        Ok(worker) => workers.push(worker),
        Err(error) => return Err((workers, error)),
    }
    match spawn_stage(
        2,
        placed_receiver,
        output_sender,
        compile,
        cancelled,
        failure,
    ) {
        Ok(worker) => workers.push(worker),
        Err(error) => return Err((workers, error)),
    }
    Ok(workers)
}

fn spawn_stage<In, Out, E, F>(
    stage: usize,
    input: mpsc::Receiver<Task<In>>,
    output: mpsc::SyncSender<Task<Out>>,
    function: F,
    cancelled: &Arc<AtomicBool>,
    failure: &Arc<Mutex<Option<PipelineError<E>>>>,
) -> Result<WorkerThread, PipelineError<E>>
where
    In: Send + 'static,
    Out: Send + 'static,
    E: Send + 'static,
    F: Fn(In) -> Result<Out, E> + Send + 'static,
{
    let worker_cancelled = Arc::clone(cancelled);
    let worker_failure = Arc::clone(failure);
    let name = format!("pipeline-stage-{stage}-worker-0");
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
        Ok(handle) => Ok(WorkerThread {
            stage: Some(stage),
            handle,
        }),
        Err(source) => Err(PipelineError::Spawn {
            stage: Some(stage),
            source,
        }),
    }
}

fn run_stage<In, Out, E, F>(
    stage: usize,
    input: mpsc::Receiver<Task<In>>,
    output: mpsc::SyncSender<Task<Out>>,
    function: F,
    cancelled: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<PipelineError<E>>>>,
) where
    F: Fn(In) -> Result<Out, E>,
{
    while !cancelled.load(Ordering::Acquire) {
        let Ok(task) = input.recv() else {
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
        let expected: Vec<_> = inputs
            .iter()
            .map(|(name, source)| {
                let mut graph = frontend::parse(source).unwrap();
                optimizer_pipeline::optimize(&mut graph);
                let graph = placement::place_graph(&graph).unwrap();
                native::compile_graph(&graph, name.clone()).unwrap()
            })
            .collect();

        let functions = compile_sources(inputs, 2).unwrap();

        assert_eq!(functions, expected);
        let names: Vec<_> = functions
            .iter()
            .map(|function| function.name.as_str())
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
    fn parse_and_placement_errors_carry_the_input_index() {
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

        let placement_error = compile_sources(
            vec![
                ("first", source(1)),
                (
                    "broken",
                    "fn first() { return 1; } fn second() { return 2; }".to_owned(),
                ),
                ("last", source(3)),
            ],
            1,
        )
        .unwrap_err();
        assert!(matches!(
            &placement_error,
            PipelinedNativeError::Compile { index: 1, .. }
        ));
        assert!(placement_error.to_string().contains("input 1"));
    }

    #[test]
    fn native_errors_carry_the_input_index() {
        let result = run_three_stages(0..4, 1, 1, Ok::<_, StageError>, Ok, |input| {
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

    #[test]
    fn rejects_zero_queue_capacities() {
        let zero_stage = run_three_stages(0..1, 0, 1, Ok::<_, &'static str>, Ok, Ok);
        assert!(matches!(
            zero_stage,
            Err(RunError::Configuration(PipelineConfigError::ZeroCapacity {
                stage: 0
            }))
        ));

        let zero_output = run_three_stages(0..1, 1, 0, Ok::<_, &'static str>, Ok, Ok);
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
        use crate::pipeline::PipelineBuilder;
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        const INPUTS: usize = 250_000;
        const ROUNDS: usize = 9;

        fn baseline(inputs: std::ops::Range<usize>) -> Vec<usize> {
            PipelineBuilder::new()
                .add_stage(1, 4, |input: usize| Ok::<_, &'static str>(input + 1))
                .add_stage(1, 4, |input: usize| Ok(input * 2))
                .add_stage(1, 4, |input: usize| Ok(input + 3))
                .output_capacity(4)
                .build()
                .unwrap()
                .run(inputs)
                .unwrap()
        }

        fn candidate(inputs: std::ops::Range<usize>) -> Vec<usize> {
            run_three_stages(
                inputs,
                4,
                4,
                |input| Ok::<_, &'static str>(input + 1),
                |input| Ok(input * 2),
                |input| Ok(input + 3),
            )
            .unwrap()
        }

        fn measure(compile: fn(std::ops::Range<usize>) -> Vec<usize>) -> (Duration, Vec<usize>) {
            let start = Instant::now();
            let outputs = compile(0..INPUTS);
            let elapsed = start.elapsed();
            black_box(&outputs);
            (elapsed, outputs)
        }

        measure(baseline);
        measure(candidate);
        let mut baseline_times = Vec::new();
        let mut candidate_times = Vec::new();
        for round in 0..ROUNDS {
            let (baseline, candidate) = if round % 2 == 0 {
                let baseline = measure(baseline);
                let candidate = measure(candidate);
                (baseline, candidate)
            } else {
                let candidate = measure(candidate);
                let baseline = measure(baseline);
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
