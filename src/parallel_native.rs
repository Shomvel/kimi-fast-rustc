use std::any::Any;
use std::error::Error;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use crate::native::{self, CompileError};
use crate::placement::{self, PlacementError};
use crate::scheduler::{Scheduler, SchedulerError, TaskResult};
use crate::son::Graph;
use crate::son_optimize;
use crate::x86::MachineFunction;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParallelCompileError {
    Scheduler(SchedulerError),
    Placement {
        index: usize,
        name: String,
        source: PlacementError,
    },
    Compile {
        index: usize,
        name: String,
        source: CompileError,
    },
    Panicked {
        index: usize,
        name: String,
        message: String,
    },
}

impl fmt::Display for ParallelCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scheduler(error) => write!(formatter, "parallel compilation failed: {error}"),
            Self::Placement {
                index,
                name,
                source,
            } => write!(
                formatter,
                "function {name} at index {index}: placement failed: {source}"
            ),
            Self::Compile {
                index,
                name,
                source,
            } => write!(
                formatter,
                "function {name} at index {index}: native compilation failed: {source}"
            ),
            Self::Panicked {
                index,
                name,
                message,
            } => write!(
                formatter,
                "function {name} at index {index}: native compilation panicked: {message}"
            ),
        }
    }
}

impl Error for ParallelCompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Scheduler(error) => Some(error),
            Self::Placement { source, .. } => Some(source),
            Self::Compile { source, .. } => Some(source),
            Self::Panicked { .. } => None,
        }
    }
}

impl From<SchedulerError> for ParallelCompileError {
    fn from(error: SchedulerError) -> Self {
        Self::Scheduler(error)
    }
}

/// Compiles independent `(function name, graph)` pairs with bounded task parallelism.
///
/// Results retain input order. Compilation errors are reported in input order as well,
/// regardless of the order in which parallel tasks finish.
pub fn compile_graphs<I, N>(
    inputs: I,
    max_parallelism: usize,
) -> Result<Vec<MachineFunction>, ParallelCompileError>
where
    I: IntoIterator<Item = (N, Graph)>,
    N: Into<String>,
{
    let inputs = inputs
        .into_iter()
        .map(|(name, graph)| (name.into(), graph))
        .collect();
    run_compilation(inputs, max_parallelism, compile_one)
}

enum CompilationFailure {
    Placement(PlacementError),
    Compile(CompileError),
    Panicked(String),
}

impl CompilationFailure {
    fn with_context(self, index: usize, name: String) -> ParallelCompileError {
        match self {
            Self::Placement(source) => ParallelCompileError::Placement {
                index,
                name,
                source,
            },
            Self::Compile(source) => ParallelCompileError::Compile {
                index,
                name,
                source,
            },
            Self::Panicked(message) => ParallelCompileError::Panicked {
                index,
                name,
                message,
            },
        }
    }
}

fn compile_one(mut graph: Graph, name: String) -> Result<MachineFunction, CompilationFailure> {
    panic::catch_unwind(AssertUnwindSafe(move || {
        son_optimize::optimize(&mut graph);
        let graph = placement::place_graph(&graph).map_err(CompilationFailure::Placement)?;
        native::compile_graph(&graph, name).map_err(CompilationFailure::Compile)
    }))
    .unwrap_or_else(|payload| Err(CompilationFailure::Panicked(panic_message(payload))))
}

fn run_compilation<F>(
    inputs: Vec<(String, Graph)>,
    max_parallelism: usize,
    compile: F,
) -> Result<Vec<MachineFunction>, ParallelCompileError>
where
    F: Fn(Graph, String) -> Result<MachineFunction, CompilationFailure> + Send + Sync + 'static,
{
    let names: Vec<_> = inputs.iter().map(|(name, _)| name.clone()).collect();
    let mut scheduler =
        Scheduler::<Result<MachineFunction, CompilationFailure>, ()>::new(max_parallelism);
    let compile = Arc::new(compile);
    for (name, graph) in inputs {
        let compile = Arc::clone(&compile);
        scheduler.add_task([], move || Ok(compile(graph, name)));
    }

    let results = scheduler.run()?;
    let mut functions = Vec::with_capacity(results.len());
    for (index, result) in results.into_iter().enumerate() {
        match result {
            TaskResult::Success(Ok(function)) => functions.push(function),
            TaskResult::Success(Err(error)) => {
                return Err(error.with_context(index, names[index].clone()));
            }
            TaskResult::Panicked(message) => {
                return Err(ParallelCompileError::Panicked {
                    index,
                    name: names[index].clone(),
                    message,
                });
            }
            TaskResult::Failed(()) | TaskResult::Cancelled => {
                unreachable!("independent compilation tasks cannot fail or be cancelled")
            }
        }
    }
    Ok(functions)
}

fn panic_message(payload: Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "compilation panicked with a non-string payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{ParallelCompileError, compile_graphs, compile_one, run_compilation};
    use crate::native::{CompileError, UnsupportedReason};
    use crate::placement::PlacementError;
    use crate::scheduler::SchedulerError;
    use crate::son::{BinaryOp, Graph, NodeId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    fn arithmetic_graph(value: i64) -> Graph {
        let mut graph = Graph::new();
        let start = graph.start();
        let parameter = graph.parameter(0);
        let constant = graph.constant(value);
        let sum = graph.binary(BinaryOp::Add, parameter, constant);
        graph.return_(start, Some(sum));
        graph
    }

    #[test]
    fn respects_parallelism_limit_and_runs_tasks_concurrently() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let first_pair = Arc::new(Barrier::new(2));
        let inputs = (0..4)
            .map(|index| (format!("function_{index}"), arithmetic_graph(index)))
            .collect();

        let functions = run_compilation(inputs, 2, {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let first_pair = Arc::clone(&first_pair);
            move |graph, name| {
                let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now_active, Ordering::SeqCst);
                if name == "function_0" || name == "function_1" {
                    first_pair.wait();
                }
                let result = compile_one(graph, name);
                active.fetch_sub(1, Ordering::SeqCst);
                result
            }
        })
        .unwrap();

        assert_eq!(functions.len(), 4);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn parallelism_one_and_many_produce_identical_results() {
        let inputs = || {
            vec![
                ("add_one".to_owned(), arithmetic_graph(1)),
                ("add_twenty".to_owned(), arithmetic_graph(20)),
                ("add_three_hundred".to_owned(), arithmetic_graph(300)),
            ]
        };

        let sequential = compile_graphs(inputs(), 1).unwrap();
        let parallel = compile_graphs(inputs(), 3).unwrap();
        assert_eq!(parallel, sequential);
    }

    #[test]
    fn collects_functions_in_input_order() {
        let inputs = vec![
            ("slow".to_owned(), arithmetic_graph(40)),
            ("fast".to_owned(), arithmetic_graph(2)),
            ("last".to_owned(), arithmetic_graph(-7)),
        ];
        let (fast_done_tx, fast_done_rx) = std::sync::mpsc::channel();
        let fast_done_rx = std::sync::Mutex::new(fast_done_rx);

        let functions = run_compilation(inputs, 2, move |graph, name| {
            if name == "slow" {
                fast_done_rx.lock().unwrap().recv().unwrap();
            }
            let is_fast = name == "fast";
            let result = compile_one(graph, name);
            if is_fast {
                fast_done_tx.send(()).unwrap();
            }
            result
        })
        .unwrap();
        let names: Vec<_> = functions
            .iter()
            .map(|function| function.name.as_str())
            .collect();
        assert_eq!(names, ["slow", "fast", "last"]);
    }

    #[test]
    fn placement_errors_include_stable_name_and_index() {
        let inputs = || {
            vec![
                ("first".to_owned(), arithmetic_graph(1)),
                ("broken".to_owned(), Graph::new()),
                ("last".to_owned(), arithmetic_graph(3)),
            ]
        };

        for parallelism in [1, 3] {
            let error = compile_graphs(inputs(), parallelism).unwrap_err();
            assert_eq!(
                error,
                ParallelCompileError::Placement {
                    index: 1,
                    name: "broken".to_owned(),
                    source: PlacementError::MissingStart,
                }
            );
            let message = error.to_string();
            assert!(message.contains("broken"), "{message}");
            assert!(message.contains("index 1"), "{message}");
        }
    }

    #[test]
    fn native_errors_include_name_and_index() {
        let mut graph = Graph::new();
        let start = graph.start();
        graph.return_(start, None::<NodeId>);

        let error = compile_graphs(vec![("void_return".to_owned(), graph)], 2).unwrap_err();
        assert_eq!(
            error,
            ParallelCompileError::Compile {
                index: 0,
                name: "void_return".to_owned(),
                source: CompileError::Unsupported {
                    node: NodeId(1),
                    reason: UnsupportedReason::ReturnWithoutValue,
                },
            }
        );
    }

    #[test]
    fn rejects_zero_parallelism() {
        let error = compile_graphs(Vec::<(String, Graph)>::new(), 0).unwrap_err();
        assert_eq!(
            error,
            ParallelCompileError::Scheduler(SchedulerError::ZeroParallelism)
        );
    }
}
