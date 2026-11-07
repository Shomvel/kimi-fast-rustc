use std::any::Any;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::error::Error;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::thread;

pub type TaskId = usize;

type TaskFn<T, E> = Box<dyn FnOnce() -> Result<T, E> + Send + 'static>;

struct Task<T, E> {
    dependencies: Vec<TaskId>,
    run: Option<TaskFn<T, E>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    ZeroParallelism,
    InvalidDependency { task: TaskId, dependency: TaskId },
    CycleDetected { cycle: Vec<TaskId> },
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroParallelism => f.write_str("parallelism must be greater than zero"),
            Self::InvalidDependency { task, dependency } => {
                write!(f, "task {task} depends on nonexistent task {dependency}")
            }
            Self::CycleDetected { cycle } => write!(f, "dependency cycle detected: {cycle:?}"),
        }
    }
}

impl Error for SchedulerError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskResult<T, E> {
    Success(T),
    Failed(E),
    Panicked(String),
    Cancelled,
}

/// Runs a fixed task graph with bounded, fail-fast parallel execution.
///
/// Once a task fails or panics, no new tasks are started. Tasks already running
/// are allowed to finish, and every task that never started is reported as
/// cancelled. Results are returned in task registration order.
pub struct Scheduler<T, E> {
    max_parallelism: usize,
    tasks: Vec<Task<T, E>>,
}

impl<T, E> Scheduler<T, E> {
    pub fn new(max_parallelism: usize) -> Self {
        Self {
            max_parallelism,
            tasks: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn add_task<F, I>(&mut self, dependencies: I, run: F) -> TaskId
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        I: IntoIterator<Item = TaskId>,
    {
        let id = self.tasks.len();
        self.tasks.push(Task {
            dependencies: dependencies.into_iter().collect(),
            run: Some(Box::new(run)),
        });
        id
    }

    pub fn run(self) -> Result<Vec<TaskResult<T, E>>, SchedulerError>
    where
        T: Send + 'static,
        E: Send + 'static,
    {
        if self.max_parallelism == 0 {
            return Err(SchedulerError::ZeroParallelism);
        }

        let dependencies = validate_graph(&self.tasks)?;
        let Self {
            max_parallelism,
            mut tasks,
        } = self;
        let task_count = tasks.len();
        if task_count == 0 {
            return Ok(Vec::new());
        }

        let mut remaining_dependencies: Vec<_> = dependencies.iter().map(Vec::len).collect();
        let mut dependents = vec![Vec::new(); task_count];
        for (task, dependencies) in dependencies.iter().enumerate() {
            for &dependency in dependencies {
                dependents[dependency].push(task);
            }
        }

        let mut ready = BinaryHeap::new();
        for (task, &count) in remaining_dependencies.iter().enumerate() {
            if count == 0 {
                ready.push(Reverse(task));
            }
        }

        let mut results: Vec<Option<TaskResult<T, E>>> =
            std::iter::repeat_with(|| None).take(task_count).collect();
        let mut handles: Vec<Option<thread::JoinHandle<()>>> =
            std::iter::repeat_with(|| None).take(task_count).collect();
        let (completed_tx, completed_rx) = mpsc::channel();
        let mut running = 0;
        let mut finished = 0;
        let mut cancelling = false;

        while finished < task_count {
            while !cancelling && running < max_parallelism {
                let Some(Reverse(task_id)) = ready.pop() else {
                    break;
                };
                let task = tasks[task_id]
                    .run
                    .take()
                    .expect("a ready task is only started once");
                let completed_tx = completed_tx.clone();
                let handle = thread::spawn(move || {
                    let result = match panic::catch_unwind(AssertUnwindSafe(task)) {
                        Ok(Ok(value)) => TaskResult::Success(value),
                        Ok(Err(error)) => TaskResult::Failed(error),
                        Err(payload) => TaskResult::Panicked(panic_message(payload)),
                    };
                    let _ = completed_tx.send((task_id, result));
                });
                handles[task_id] = Some(handle);
                running += 1;
            }

            if running == 0 {
                debug_assert_eq!(finished, task_count);
                break;
            }

            let (task_id, result) = completed_rx
                .recv()
                .expect("every running task sends exactly one result");
            running -= 1;
            if let Some(handle) = handles[task_id].take() {
                let _ = handle.join();
            }

            let failed = matches!(result, TaskResult::Failed(_) | TaskResult::Panicked(_));
            results[task_id] = Some(result);
            finished += 1;

            if failed && !cancelling {
                cancelling = true;
                ready.clear();
                for pending_id in 0..task_count {
                    if results[pending_id].is_none() && handles[pending_id].is_none() {
                        results[pending_id] = Some(TaskResult::Cancelled);
                        finished += 1;
                    }
                }
            } else if !failed && !cancelling {
                for &dependent in &dependents[task_id] {
                    remaining_dependencies[dependent] -= 1;
                    if remaining_dependencies[dependent] == 0 {
                        ready.push(Reverse(dependent));
                    }
                }
            }
        }

        Ok(results
            .into_iter()
            .map(|result| result.expect("every task has a terminal result"))
            .collect())
    }
}

fn validate_graph<T, E>(tasks: &[Task<T, E>]) -> Result<Vec<Vec<TaskId>>, SchedulerError> {
    let mut dependencies = Vec::with_capacity(tasks.len());
    for (task, definition) in tasks.iter().enumerate() {
        for &dependency in &definition.dependencies {
            if dependency >= tasks.len() {
                return Err(SchedulerError::InvalidDependency { task, dependency });
            }
        }
        let mut normalized = definition.dependencies.clone();
        normalized.sort_unstable();
        normalized.dedup();
        dependencies.push(normalized);
    }

    let mut states = vec![0_u8; tasks.len()];
    let mut stack = Vec::new();
    for task in 0..tasks.len() {
        if states[task] == 0 {
            visit(task, &dependencies, &mut states, &mut stack)?;
        }
    }
    Ok(dependencies)
}

fn visit(
    task: TaskId,
    dependencies: &[Vec<TaskId>],
    states: &mut [u8],
    stack: &mut Vec<TaskId>,
) -> Result<(), SchedulerError> {
    states[task] = 1;
    stack.push(task);

    for &dependency in &dependencies[task] {
        match states[dependency] {
            0 => visit(dependency, dependencies, states, stack)?,
            1 => {
                let cycle_start = stack
                    .iter()
                    .position(|&task| task == dependency)
                    .expect("an active task is on the DFS stack");
                let mut cycle = stack[cycle_start..].to_vec();
                cycle.push(dependency);
                return Err(SchedulerError::CycleDetected { cycle });
            }
            _ => {}
        }
    }

    stack.pop();
    states[task] = 2;
    Ok(())
}

fn panic_message(payload: Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "task panicked with a non-string payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{Scheduler, SchedulerError, TaskResult};
    use std::panic;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn honors_dependencies_and_parallelism_limit() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let mut scheduler = Scheduler::<usize, ()>::new(2);

        for id in 0..2 {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let completed = Arc::clone(&completed);
            scheduler.add_task([], move || {
                let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now_active, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(20));
                completed.lock().unwrap().push(id);
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(id)
            });
        }

        let completed_for_child = Arc::clone(&completed);
        let child = scheduler.add_task([0, 1], move || {
            let completed = completed_for_child.lock().unwrap();
            assert!(completed.contains(&0));
            assert!(completed.contains(&1));
            Ok(2)
        });
        assert_eq!(child, 2);

        let results = scheduler.run().unwrap();
        assert_eq!(
            results,
            vec![
                TaskResult::Success(0),
                TaskResult::Success(1),
                TaskResult::Success(2)
            ]
        );
        assert!(peak.load(Ordering::SeqCst) <= 2);
    }

    #[test]
    fn collects_results_in_registration_order() {
        let mut scheduler = Scheduler::<usize, ()>::new(3);
        for (id, delay_ms) in [(0, 30), (1, 0), (2, 10)] {
            scheduler.add_task([], move || {
                thread::sleep(Duration::from_millis(delay_ms));
                Ok(id)
            });
        }

        assert_eq!(
            scheduler.run().unwrap(),
            vec![
                TaskResult::Success(0),
                TaskResult::Success(1),
                TaskResult::Success(2)
            ]
        );
    }

    #[test]
    fn failure_cancels_all_tasks_that_have_not_started() {
        let ran = Arc::new(AtomicUsize::new(0));
        let mut scheduler = Scheduler::<(), &'static str>::new(1);
        scheduler.add_task([], || Err("compile failed"));
        for _ in 0..2 {
            let ran = Arc::clone(&ran);
            scheduler.add_task([], move || {
                ran.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });
        }

        assert_eq!(
            scheduler.run().unwrap(),
            vec![
                TaskResult::Failed("compile failed"),
                TaskResult::Cancelled,
                TaskResult::Cancelled
            ]
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn running_tasks_finish_after_another_task_fails() {
        let barrier = Arc::new(Barrier::new(2));
        let mut scheduler = Scheduler::<usize, ()>::new(2);

        let failing_barrier = Arc::clone(&barrier);
        scheduler.add_task([], move || {
            failing_barrier.wait();
            Err(())
        });
        let running_barrier = Arc::clone(&barrier);
        scheduler.add_task([], move || {
            running_barrier.wait();
            thread::sleep(Duration::from_millis(20));
            Ok(7)
        });
        scheduler.add_task([], || Ok(8));

        assert_eq!(
            scheduler.run().unwrap(),
            vec![
                TaskResult::Failed(()),
                TaskResult::Success(7),
                TaskResult::Cancelled
            ]
        );
    }

    #[test]
    fn catches_panics_and_cancels_pending_tasks() {
        let mut scheduler = Scheduler::<(), ()>::new(1);
        scheduler.add_task([], || panic!("worker panic"));
        scheduler.add_task([], || Ok(()));

        let results = scheduler.run().unwrap();
        assert_eq!(
            results,
            vec![
                TaskResult::Panicked("worker panic".to_owned()),
                TaskResult::Cancelled
            ]
        );
    }

    #[test]
    fn reports_non_string_panics() {
        let mut scheduler = Scheduler::<(), ()>::new(1);
        scheduler.add_task([], || panic::panic_any(42_u32));

        assert_eq!(
            scheduler.run().unwrap(),
            vec![TaskResult::Panicked(
                "task panicked with a non-string payload".to_owned()
            )]
        );
    }

    #[test]
    fn detects_cycles_before_running_tasks() {
        let ran = Arc::new(AtomicUsize::new(0));
        let mut scheduler = Scheduler::<(), ()>::new(2);
        let first_ran = Arc::clone(&ran);
        scheduler.add_task([1], move || {
            first_ran.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let second_ran = Arc::clone(&ran);
        scheduler.add_task([0], move || {
            second_ran.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

        assert_eq!(
            scheduler.run(),
            Err(SchedulerError::CycleDetected {
                cycle: vec![0, 1, 0]
            })
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rejects_missing_dependencies_before_running_tasks() {
        let ran = Arc::new(AtomicUsize::new(0));
        let mut scheduler = Scheduler::<(), ()>::new(1);
        let task_ran = Arc::clone(&ran);
        scheduler.add_task([], move || {
            task_ran.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        scheduler.add_task([9], || Ok(()));

        assert_eq!(
            scheduler.run(),
            Err(SchedulerError::InvalidDependency {
                task: 1,
                dependency: 9
            })
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn accepts_duplicate_dependency_edges() {
        let mut scheduler = Scheduler::<usize, ()>::new(2);
        scheduler.add_task([], || Ok(1));
        scheduler.add_task([0, 0], || Ok(2));

        assert_eq!(
            scheduler.run().unwrap(),
            vec![TaskResult::Success(1), TaskResult::Success(2)]
        );
    }

    #[test]
    fn rejects_zero_parallelism() {
        let scheduler = Scheduler::<(), ()>::new(0);
        assert_eq!(scheduler.run(), Err(SchedulerError::ZeroParallelism));
    }

    #[test]
    fn empty_graph_has_no_results() {
        let scheduler = Scheduler::<(), ()>::new(4);
        assert!(scheduler.is_empty());
        assert_eq!(scheduler.run().unwrap(), Vec::new());
    }
}
