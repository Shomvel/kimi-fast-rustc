//! Bounded workers for owned backend jobs. The caller keeps its implicit
//! jobserver token; each additional worker owns a borrowed token until joined.
use std::sync::{Mutex, mpsc};

enum Admission<Token> {
    Token(Token),
    Finished,
}

struct FinishAdmission<Token>(mpsc::Sender<Admission<Token>>);

impl<Token> Drop for FinishAdmission<Token> {
    fn drop(&mut self) {
        let _ = self.0.send(Admission::Finished);
    }
}

pub(crate) fn thread_limit() -> Result<usize, String> {
    let limit = std::thread::available_parallelism().map_or(1, usize::from);
    match std::env::var("KFRC_CODEGEN_THREADS") {
        Err(std::env::VarError::NotPresent) => Ok(limit),
        Ok(value) => value
            .parse::<std::num::NonZeroUsize>()
            .map(|requested| requested.get().min(limit))
            .map_err(|_| "KFRC_CODEGEN_THREADS must be a positive integer".into()),
        Err(_) => Err("KFRC_CODEGEN_THREADS must be a positive integer".into()),
    }
}

pub(crate) fn map<T: Send, U: Send>(
    jobs: Vec<T>,
    limit: usize,
    run: impl Fn(T) -> U + Sync,
) -> Vec<U> {
    if limit <= 1 || jobs.len() <= 1 {
        return jobs.into_iter().map(run).collect();
    }
    map_with_client(jobs, limit, rustc_data_structures::jobserver::client(), run)
}

fn map_with_client<T: Send, U: Send>(
    jobs: Vec<T>,
    limit: usize,
    client: rustc_data_structures::jobserver::Client,
    run: impl Fn(T) -> U + Sync,
) -> Vec<U> {
    let workers = limit.min(jobs.len()).saturating_sub(1);
    if workers == 0 {
        return jobs.into_iter().map(run).collect();
    }
    let (send, tokens) = mpsc::channel();
    let finish = send.clone();
    // The helper supports inherited anonymous pipes as well as FIFO jobservers.
    // Acquiring on the caller could deadlock Cargo -j1: the caller must keep
    // processing jobs while no extra tokens are available.
    let helper = client
        .into_helper_thread(move |token| {
            if let Ok(token) = token {
                let _ = send.send(Admission::Token(token));
            }
        })
        .expect("spawn kfrc jobserver helper");
    for _ in 0..workers {
        helper.request_token();
    }
    let results = map_with_tokens(jobs, tokens, finish, run);
    // Drop cancels outstanding requests and joins the helper, including when
    // all work finished on the caller without obtaining a token.
    drop(helper);
    results
}

fn map_with_tokens<T: Send, U: Send, Token: Send>(
    jobs: Vec<T>,
    tokens: mpsc::Receiver<Admission<Token>>,
    finish: mpsc::Sender<Admission<Token>>,
    run: impl Fn(T) -> U + Sync,
) -> Vec<U> {
    let jobs = Mutex::new(jobs.into_iter().enumerate());
    let work = || {
        let mut results = Vec::new();
        loop {
            let job = jobs.lock().unwrap().next();
            let Some((index, job)) = job else { break };
            results.push((index, run(job)));
        }
        results
    };
    let mut results = std::thread::scope(|scope| {
        let work = &work;
        let jobs = &jobs;
        // Admission must progress while the implicit-token caller compiles a
        // long function. This thread waits for tokens, never processes a job.
        let admission = std::thread::Builder::new()
            .name("kfrc-admission".into())
            .spawn_scoped(scope, move || {
                let mut workers = Vec::new();
                while let Ok(Admission::Token(token)) = tokens.recv() {
                    if jobs.lock().unwrap().len() == 0 {
                        break;
                    }
                    workers.push(
                        std::thread::Builder::new()
                            .name("kfrc-codegen".into())
                            .stack_size(8 * 1024 * 1024)
                            .spawn_scoped(scope, move || {
                                let _token = token;
                                work()
                            })
                            .expect("spawn kfrc codegen worker"),
                    );
                }
                let mut results = Vec::new();
                for worker in workers {
                    results.extend(worker.join().unwrap());
                }
                results
            })
            .expect("spawn kfrc token admission");
        // Send completion even when the caller panics, so scope can join an
        // admission thread still waiting on an exhausted Cargo -j1 jobserver.
        let finish = FinishAdmission(finish);
        let mut results = work();
        drop(finish);
        results.extend(admission.join().unwrap());
        results
    });
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn empty_jobserver_finishes_on_caller_without_waiting() {
        let client = rustc_data_structures::jobserver::Client::new(0).unwrap();
        let caller = std::thread::current().id();
        let results = map_with_client((0..20).collect(), 8, client, |job| {
            assert_eq!(std::thread::current().id(), caller);
            job
        });
        assert_eq!(results, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn one_extra_token_allows_only_one_extra_active_worker() {
        let client = rustc_data_structures::jobserver::Client::new(1).unwrap();
        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        map_with_client((0..100).collect(), 8, client.clone(), |_| {
            let n = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(n, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(1));
            active.fetch_sub(1, Ordering::SeqCst);
        });
        assert_eq!(maximum.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(client.available().unwrap(), 1);
    }

    #[test]
    fn serial_and_parallel_preserve_order_and_process_each_job_once() {
        for workers in [0, 1, 3] {
            let visits: Vec<_> = (0..100).map(|_| AtomicUsize::new(0)).collect();
            let (send, tokens) = mpsc::channel();
            for _ in 0..workers {
                send.send(Admission::Token(())).unwrap();
            }
            let results = map_with_tokens((0..100).collect(), tokens, send, |job| {
                visits[job].fetch_add(1, Ordering::Relaxed);
                if job % 3 == 0 {
                    std::thread::yield_now();
                }
                job * 2
            });
            assert_eq!(results, (0..100).map(|n| n * 2).collect::<Vec<_>>());
            assert!(visits.iter().all(|n| n.load(Ordering::Relaxed) == 1));
        }
    }

    #[test]
    fn panic_joins_workers_and_releases_all_tokens() {
        struct Token(Arc<AtomicUsize>);
        impl Drop for Token {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let released = Arc::new(AtomicUsize::new(0));
        let (send, tokens) = mpsc::channel();
        for _ in 0..3 {
            send.send(Admission::Token(Token(released.clone())))
                .unwrap();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            map_with_tokens(vec![0, 1, 2, 3], tokens, send, |job| {
                assert_ne!(job, 1);
            })
        }));
        assert!(result.is_err());
        assert_eq!(released.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn caller_panic_cancels_admission_without_an_available_token() {
        let client = rustc_data_structures::jobserver::Client::new(0).unwrap();
        let result = std::panic::catch_unwind(|| {
            map_with_client(vec![0, 1], 8, client.clone(), |_| panic!("caller failed"));
        });
        assert!(result.is_err());
        assert_eq!(client.available().unwrap(), 0);
    }

    #[test]
    fn a_late_token_starts_a_worker_before_the_caller_finishes_its_job() {
        let (send, tokens) = mpsc::channel();
        let finish = send.clone();
        let (started, first_job) = mpsc::channel();
        let (second, second_job) = mpsc::channel();
        let second_job = Mutex::new(second_job);
        std::thread::scope(|scope| {
            scope.spawn(move || {
                first_job
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
                send.send(Admission::Token(())).unwrap();
            });
            let results = map_with_tokens(vec![0, 1], tokens, finish, |job| {
                if job == 0 {
                    started.send(()).unwrap();
                    second_job
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .unwrap();
                } else {
                    second.send(()).unwrap();
                }
                job
            });
            assert_eq!(results, [0, 1]);
        });
    }
}
