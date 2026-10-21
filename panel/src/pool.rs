//! The workers that answer the requests.
//!
//! The server hands over a request, and a worker takes it from the queue
//! and answers it. There are a few workers, so a slow request does not
//! hold up the others, and the queue has a limit, so a flood of requests
//! cannot use all the memory of the phone. When the queue is full the
//! server answers `503` right away.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::app::{self, App};
use crate::web::WebRequest;

/// A job that waits for a worker.
pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// The state the workers share.
#[derive(Default)]
struct Shared {
    /// The requests that are waiting for a worker.
    queue: Mutex<VecDeque<Job>>,
    /// Wakes the workers up when there is something to do.
    ready: Condvar,
    /// Told to the workers when the pool is closed.
    closing: AtomicBool,
}

/// A pool of workers that answer the requests of the panel.
pub struct Pool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
    /// How many requests may wait for a worker.
    capacity: usize,
}

impl Pool {
    /// Start a pool with a number of workers and a queue of a size.
    pub fn new(workers: usize, capacity: usize) -> Pool {
        let shared = Arc::new(Shared::default());
        let mut handles = Vec::new();

        for _ in 0..workers {
            let shared = shared.clone();
            handles.push(std::thread::spawn(move || {
                worker(shared);
            }));
        }

        Pool {
            shared,
            workers: handles,
            capacity,
        }
    }

    /// The number of workers of the pool.
    pub fn workers(&self) -> usize {
        self.workers.len()
    }

    /// Give a job to the pool.
    ///
    /// It answers `false` when the queue is full, so the caller can tell
    /// the visitor that the panel is busy.
    pub fn submit(&self, job: Job) -> bool {
        let mut queue = self.lock();

        if queue.len() >= self.capacity {
            return false;
        }

        queue.push_back(job);
        self.shared.ready.notify_one();
        true
    }

    /// Give a request to the pool.
    ///
    /// When the queue is full the request is handed back, so the caller can
    /// answer that the panel is busy. The check and the putting in the
    /// queue happen under the same lock, so no request is ever lost.
    pub fn submit_request(
        &self,
        app: Arc<App>,
        request: tiny_http::Request,
    ) -> Result<(), Box<tiny_http::Request>> {
        let mut queue = self.lock();

        if queue.len() >= self.capacity {
            return Err(Box::new(request));
        }

        queue.push_back(Box::new(move || serve_request(&app, request)));
        self.shared.ready.notify_one();
        Ok(())
    }

    /// The queue of the pool, locked.
    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Job>> {
        match self.shared.queue.lock() {
            Ok(queue) => queue,
            // A queue that was poisoned is still usable.
            Err(err) => err.into_inner(),
        }
    }

    /// Stop the pool and wait for the workers to finish.
    pub fn shutdown(&mut self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        self.shared.ready.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The work of one worker.
fn worker(shared: Arc<Shared>) {
    loop {
        let job = {
            let mut queue = match shared.queue.lock() {
                Ok(queue) => queue,
                Err(err) => err.into_inner(),
            };

            while queue.is_empty() && !shared.closing.load(Ordering::Relaxed) {
                queue = match shared.ready.wait(queue) {
                    Ok(queue) => queue,
                    Err(err) => err.into_inner(),
                };
            }

            if queue.is_empty() {
                // The pool is closing and there is nothing left to do.
                return;
            }

            queue.pop_front()
        };

        if let Some(job) = job {
            // A panic in a request must not take the panel down, so it is
            // caught here and the worker goes on with the next one.
            let _ = catch_unwind(AssertUnwindSafe(job));
        }
    }
}

/// Answer a request of the panel in a worker.
///
/// The request is read and answered here, so a slow visitor does not hold
/// up the others.
pub fn serve_request(app: &App, request: tiny_http::Request) {
    let request = WebRequest::new(request);
    let response = app::handle(app, &request);
    let _ = request.raw.respond(response);
}

/// The answer when the panel is too busy.
pub fn busy() -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    crate::web::html(
        503,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Busy - noobs-panel</title></head><body>\
         <h1>The panel is busy</h1><p>Too many requests at once. Try again in a moment.</p>\
         </body></html>"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_pool_answers_the_jobs() {
        let pool = Pool::new(2, 8);
        let counter = Arc::new(Mutex::new(0));

        for _ in 0..8 {
            let counter = counter.clone();
            assert!(pool.submit(Box::new(move || {
                *counter.lock().unwrap() += 1;
            })));
        }

        // Wait for the workers to be done.
        for _ in 0..100 {
            if *counter.lock().unwrap() == 8 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(*counter.lock().unwrap(), 8);
    }

    #[test]
    fn the_queue_has_a_limit() {
        let pool = Pool::new(1, 2);
        let release = Arc::new((Mutex::new(false), Condvar::new()));

        // The first job holds the worker, the next two fill the queue.
        let hold = release.clone();
        assert!(pool.submit(Box::new(move || {
            let (lock, ready) = &*hold;
            let mut busy = lock.lock().unwrap();
            while !*busy {
                busy = ready.wait(busy).unwrap();
            }
        })));

        std::thread::sleep(Duration::from_millis(50));
        assert!(pool.submit(Box::new(|| {})));
        assert!(pool.submit(Box::new(|| {})));

        // The queue is full now.
        assert!(!pool.submit(Box::new(|| {})));

        // Let the worker go.
        let (lock, ready) = &*release;
        *lock.lock().unwrap() = true;
        ready.notify_all();
    }

    #[test]
    fn a_panic_does_not_stop_the_worker() {
        let pool = Pool::new(1, 8);
        let counter = Arc::new(Mutex::new(0));

        assert!(pool.submit(Box::new(|| panic!("the request went wrong"))));

        let counter2 = counter.clone();
        assert!(pool.submit(Box::new(move || {
            *counter2.lock().unwrap() += 1;
        })));

        for _ in 0..100 {
            if *counter.lock().unwrap() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(*counter.lock().unwrap(), 1);
    }
}
