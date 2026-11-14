use std::any::Any;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};

type Value = Box<dyn Any + Send + 'static>;
type StageFn<E> = dyn Fn(Value) -> Result<Value, E> + Send + Sync + 'static;

struct Task {
    index: usize,
    value: Value,
}

struct Stage<E> {
    parallelism: usize,
    capacity: usize,
    function: Arc<StageFn<E>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PipelineConfigError {
    ZeroParallelism { stage: usize },
    ZeroCapacity { stage: usize },
    ZeroOutputCapacity,
}

impl fmt::Display for PipelineConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroParallelism { stage } => {
                write!(formatter, "pipeline stage {stage} has zero parallelism")
            }
            Self::ZeroCapacity { stage } => {
                write!(formatter, "pipeline stage {stage} has zero queue capacity")
            }
            Self::ZeroOutputCapacity => {
                formatter.write_str("pipeline output queue has zero capacity")
            }
        }
    }
}

impl Error for PipelineConfigError {}

#[derive(Debug)]
pub enum PipelineError<E> {
    Stage {
        stage: usize,
        index: usize,
        source: E,
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

impl<E> fmt::Display for PipelineError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stage {
                stage,
                index,
                source: _,
            } => write!(formatter, "pipeline stage {stage} rejected input {index}"),
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

impl<E: Error + 'static> Error for PipelineError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Stage { source, .. } => Some(source),
            Self::Spawn { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Builds a typed sequence of independently parallel, bounded compiler stages.
///
/// `In` is the original input type and `Out` is the current stage output type.
/// Queue capacities apply to stage inputs; `output_capacity` bounds the final
/// result queue. Every stage and the output queue default to backpressure rather
/// than buffering an unbounded number of tasks.
pub struct PipelineBuilder<In, E, Out = In> {
    stages: Vec<Stage<E>>,
    output_capacity: usize,
    marker: PhantomData<fn(In) -> Out>,
}

impl<In, E> PipelineBuilder<In, E> {
    pub fn new() -> Self {
        Self {
            stages: Vec::new(),
            output_capacity: 1,
            marker: PhantomData,
        }
    }
}

impl<In, E> Default for PipelineBuilder<In, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<In, E, Out> PipelineBuilder<In, E, Out> {
    pub fn output_capacity(mut self, capacity: usize) -> Self {
        self.output_capacity = capacity;
        self
    }

    pub fn add_stage<Next, F>(
        mut self,
        parallelism: usize,
        capacity: usize,
        function: F,
    ) -> PipelineBuilder<In, E, Next>
    where
        Out: Send + 'static,
        Next: Send + 'static,
        F: Fn(Out) -> Result<Next, E> + Send + Sync + 'static,
    {
        let function = move |value: Value| {
            let value = value.downcast::<Out>().unwrap_or_else(|_| {
                panic!("pipeline stage input type does not match its builder type")
            });
            function(*value).map(|value| Box::new(value) as Value)
        };
        self.stages.push(Stage {
            parallelism,
            capacity,
            function: Arc::new(function),
        });
        PipelineBuilder {
            stages: self.stages,
            output_capacity: self.output_capacity,
            marker: PhantomData,
        }
    }

    pub fn build(self) -> Result<Pipeline<In, Out, E>, PipelineConfigError> {
        for (index, stage) in self.stages.iter().enumerate() {
            if stage.parallelism == 0 {
                return Err(PipelineConfigError::ZeroParallelism { stage: index });
            }
            if stage.capacity == 0 {
                return Err(PipelineConfigError::ZeroCapacity { stage: index });
            }
        }
        if self.output_capacity == 0 {
            return Err(PipelineConfigError::ZeroOutputCapacity);
        }
        Ok(Pipeline {
            stages: self.stages,
            output_capacity: self.output_capacity,
            marker: PhantomData,
        })
    }
}

/// A reusable multi-stage pipeline with stable output ordering and fail-fast
/// cancellation. Running stage functions are allowed to finish; queued work is
/// discarded after an error or panic. All spawned threads are joined before
/// `run` returns.
pub struct Pipeline<In, Out, E> {
    stages: Vec<Stage<E>>,
    output_capacity: usize,
    marker: PhantomData<fn(In) -> Out>,
}

impl<In, Out, E> Pipeline<In, Out, E> {
    pub fn run<I>(&self, inputs: I) -> Result<Vec<Out>, PipelineError<E>>
    where
        In: Send + 'static,
        Out: Send + 'static,
        E: Send + 'static,
        I: IntoIterator<Item = In>,
        I::IntoIter: Send + 'static,
    {
        let cancelled = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        let submitted = Arc::new(AtomicUsize::new(0));
        let mut senders = Vec::with_capacity(self.stages.len() + 1);
        let mut receivers = Vec::with_capacity(self.stages.len() + 1);

        for stage in &self.stages {
            let (sender, receiver) = mpsc::sync_channel(stage.capacity);
            senders.push(Some(sender));
            receivers.push(Some(receiver));
        }
        let (output_sender, output_receiver) = mpsc::sync_channel(self.output_capacity);
        senders.push(Some(output_sender));
        receivers.push(Some(output_receiver));

        let input_sender = senders[0]
            .take()
            .expect("the pipeline always has an input channel");
        let output_receiver = receivers
            .pop()
            .expect("the pipeline always has an output channel")
            .expect("the output receiver is present before workers start");
        let input_iterator = inputs.into_iter();
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
                    let task = Task {
                        index,
                        value: Box::new(value),
                    };
                    if input_sender.send(task).is_err() {
                        break;
                    }
                    input_submitted.fetch_add(1, Ordering::Release);
                    index += 1;
                }
            });

        let input_handle = match input_handle {
            Ok(handle) => handle,
            Err(source) => {
                return Err(PipelineError::Spawn {
                    stage: None,
                    source,
                });
            }
        };
        let mut threads = vec![WorkerThread {
            stage: None,
            handle: input_handle,
        }];

        'spawn_stages: for (stage_index, definition) in self.stages.iter().enumerate() {
            let input = Arc::new(Mutex::new(
                receivers[stage_index]
                    .take()
                    .expect("each stage receiver is taken once"),
            ));
            let output = senders[stage_index + 1]
                .take()
                .expect("each stage output sender is taken once");

            for worker_index in 0..definition.parallelism {
                let input = Arc::clone(&input);
                let output = output.clone();
                let function = Arc::clone(&definition.function);
                let worker_cancelled = Arc::clone(&cancelled);
                let worker_failure = Arc::clone(&failure);
                let name = format!("pipeline-stage-{stage_index}-worker-{worker_index}");
                let handle = thread::Builder::new().name(name).spawn(move || {
                    run_stage(
                        stage_index,
                        input,
                        output,
                        function,
                        worker_cancelled,
                        worker_failure,
                    );
                });
                match handle {
                    Ok(handle) => threads.push(WorkerThread {
                        stage: Some(stage_index),
                        handle,
                    }),
                    Err(source) => {
                        record_failure(
                            &failure,
                            &cancelled,
                            PipelineError::Spawn {
                                stage: Some(stage_index),
                                source,
                            },
                        );
                        break 'spawn_stages;
                    }
                }
            }
        }

        drop(senders);
        drop(receivers);

        let mut outputs = BTreeMap::new();
        let mut duplicate_output = false;
        while !cancelled.load(Ordering::Acquire) {
            match output_receiver.recv() {
                Ok(task) => {
                    if outputs.insert(task.index, task.value).is_some() {
                        duplicate_output = true;
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        drop(output_receiver);
        cancelled.store(true, Ordering::Release);
        if duplicate_output {
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
            return Err(error);
        }
        if outputs.len() != submitted.load(Ordering::Acquire) {
            return Err(PipelineError::Disconnected);
        }

        Ok(outputs
            .into_values()
            .map(|value| {
                *value.downcast::<Out>().unwrap_or_else(|_| {
                    panic!("pipeline output type does not match its builder type")
                })
            })
            .collect())
    }
}

struct WorkerThread {
    stage: Option<usize>,
    handle: JoinHandle<()>,
}

fn run_stage<E>(
    stage_index: usize,
    input: Arc<Mutex<mpsc::Receiver<Task>>>,
    output: mpsc::SyncSender<Task>,
    function: Arc<StageFn<E>>,
    cancelled: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<PipelineError<E>>>>,
) {
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
                        stage: stage_index,
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
                        stage: stage_index,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;
    use std::sync::{Barrier, Condvar};
    use std::time::Duration;

    #[derive(Debug, Eq, PartialEq)]
    struct Graph(usize);

    #[derive(Debug, Eq, PartialEq)]
    struct Code(usize);

    #[test]
    fn heterogeneous_stages_collect_in_input_order() {
        let pipeline = PipelineBuilder::new()
            .add_stage(4, 2, |input: usize| {
                Ok::<_, &'static str>(Graph(input * 2 + 1))
            })
            .add_stage(3, 1, |Graph(mut value)| {
                let delay = (7 - value % 7) as u64;
                thread::sleep(Duration::from_micros(delay * 100));
                value = value.wrapping_mul(3);
                Ok(value)
            })
            .add_stage(2, 3, |value| Ok(Code(value + 7)))
            .output_capacity(2)
            .build()
            .unwrap();

        let outputs = pipeline.run(0..80).unwrap();
        let expected: Vec<_> = (0_usize..80)
            .map(|input| Code((input * 2 + 1).wrapping_mul(3) + 7))
            .collect();
        assert_eq!(outputs, expected);
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
    fn high_concurrency_is_bounded_and_stable() {
        const TASKS: usize = 512;
        const FIRST_WORKERS: usize = 8;

        let first_active = Arc::new(AtomicUsize::new(0));
        let first_peak = Arc::new(AtomicUsize::new(0));
        let second_active = Arc::new(AtomicUsize::new(0));
        let second_peak = Arc::new(AtomicUsize::new(0));
        let third_active = Arc::new(AtomicUsize::new(0));
        let third_peak = Arc::new(AtomicUsize::new(0));
        let first_workers_barrier = Arc::new(Barrier::new(FIRST_WORKERS));
        let first_active_stage = Arc::clone(&first_active);
        let first_peak_stage = Arc::clone(&first_peak);
        let second_active_stage = Arc::clone(&second_active);
        let second_peak_stage = Arc::clone(&second_peak);
        let third_active_stage = Arc::clone(&third_active);
        let third_peak_stage = Arc::clone(&third_peak);
        let stage_barrier = Arc::clone(&first_workers_barrier);

        let pipeline = PipelineBuilder::new()
            .add_stage(FIRST_WORKERS, 8, move |input: usize| {
                let _guard = enter_stage(&first_active_stage, &first_peak_stage);
                if input < FIRST_WORKERS {
                    stage_barrier.wait();
                }
                Ok::<_, &'static str>(input * 3)
            })
            .add_stage(5, 3, move |input| {
                let _guard = enter_stage(&second_active_stage, &second_peak_stage);
                thread::sleep(Duration::from_micros(50));
                Ok(input + 5)
            })
            .add_stage(6, 2, move |input| {
                let _guard = enter_stage(&third_active_stage, &third_peak_stage);
                thread::sleep(Duration::from_micros(50));
                Ok(input ^ 1)
            })
            .output_capacity(4)
            .build()
            .unwrap();

        let outputs = pipeline.run(0..TASKS).unwrap();
        let expected: Vec<_> = (0..TASKS).map(|input| (input * 3 + 5) ^ 1).collect();
        assert_eq!(outputs, expected);
        assert_eq!(first_peak.load(Ordering::Relaxed), FIRST_WORKERS);
        assert!(second_peak.load(Ordering::Relaxed) <= 5);
        assert!(third_peak.load(Ordering::Relaxed) <= 6);
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
    fn bounded_input_queue_applies_backpressure_to_the_producer() {
        let (next_tx, next_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let stage_gate = Arc::clone(&gate);
        let pipeline = PipelineBuilder::new()
            .add_stage(1, 2, move |input: usize| {
                started_tx.send(input).unwrap();
                let (lock, condition) = &*stage_gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = condition.wait(released).unwrap();
                }
                Ok::<_, &'static str>(input)
            })
            .build()
            .unwrap();
        let inputs = ObservedInputs {
            range: 0..100,
            next: next_tx,
        };

        let runner = thread::spawn(move || pipeline.run(inputs));
        assert_eq!(next_rx.recv_timeout(Duration::from_secs(1)), Ok(0));
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(1)), Ok(0));
        assert_eq!(next_rx.recv_timeout(Duration::from_secs(1)), Ok(1));
        assert_eq!(next_rx.recv_timeout(Duration::from_secs(1)), Ok(2));
        assert_eq!(next_rx.recv_timeout(Duration::from_secs(1)), Ok(3));
        let blocked = next_rx.recv_timeout(Duration::from_millis(75));

        let (lock, condition) = &*gate;
        *lock.lock().unwrap() = true;
        condition.notify_all();
        let outputs = runner.join().unwrap().unwrap();

        assert!(matches!(blocked, Err(mpsc::RecvTimeoutError::Timeout)));
        assert_eq!(outputs, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn stage_error_stops_new_work_and_returns_without_deadlock() {
        const TASKS: usize = 1_024;

        let consumed = Arc::new(AtomicUsize::new(0));
        let consumed_by_iterator = Arc::clone(&consumed);
        let codegen_started = Arc::new(AtomicUsize::new(0));
        let codegen_started_by_stage = Arc::clone(&codegen_started);
        let pipeline = PipelineBuilder::new()
            .add_stage(2, 2, |input: usize| Ok::<_, &'static str>(input))
            .add_stage(1, 1, |input| {
                if input == 0 {
                    Err("graph verification failed")
                } else {
                    Ok(input)
                }
            })
            .add_stage(4, 2, move |input| {
                codegen_started_by_stage.fetch_add(1, Ordering::Relaxed);
                Ok(input)
            })
            .build()
            .unwrap();
        let inputs = (0..TASKS).inspect(move |_| {
            consumed_by_iterator.fetch_add(1, Ordering::Relaxed);
        });

        let (done_tx, done_rx) = mpsc::channel();
        let runner = thread::spawn(move || {
            done_tx.send(pipeline.run(inputs)).unwrap();
        });
        let result = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("pipeline did not stop after a stage error");
        runner.join().unwrap();

        assert!(matches!(
            result,
            Err(PipelineError::Stage {
                stage: 1,
                index: 0,
                source: "graph verification failed",
            })
        ));
        assert!(consumed.load(Ordering::Relaxed) < TASKS);
        assert!(codegen_started.load(Ordering::Relaxed) < TASKS);
    }

    #[test]
    fn stage_panic_propagates_and_joins_other_workers() {
        let pipeline = PipelineBuilder::new()
            .add_stage(4, 2, |input: usize| Ok::<_, &'static str>(input + 1))
            .add_stage(2, 1, |input| {
                if input == 4 {
                    panic!("optimizer exploded");
                }
                Ok(input)
            })
            .add_stage(2, 1, |input| Ok(input * 2))
            .build()
            .unwrap();

        let (done_tx, done_rx) = mpsc::channel();
        let runner = thread::spawn(move || {
            done_tx.send(pipeline.run(0..128)).unwrap();
        });
        let result = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("pipeline did not stop after a panic");
        runner.join().unwrap();

        match result {
            Err(PipelineError::Panicked {
                stage: 1,
                index: 3,
                message,
            }) => assert_eq!(message, "optimizer exploded"),
            other => panic!("unexpected pipeline result: {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_configuration_and_supports_identity_pipeline() {
        let zero_workers = PipelineBuilder::<usize, &'static str>::new()
            .add_stage(0, 1, Ok::<_, &'static str>)
            .build();
        assert!(matches!(
            zero_workers,
            Err(PipelineConfigError::ZeroParallelism { stage: 0 })
        ));

        let zero_capacity = PipelineBuilder::<usize, &'static str>::new()
            .add_stage(1, 0, Ok::<_, &'static str>)
            .build();
        assert!(matches!(
            zero_capacity,
            Err(PipelineConfigError::ZeroCapacity { stage: 0 })
        ));

        let identity = PipelineBuilder::<usize, &'static str>::new()
            .build()
            .unwrap();
        assert_eq!(identity.run(0..16).unwrap(), (0..16).collect::<Vec<_>>());
    }
}
