//! Run slow work (a scan, a copy) off the UI thread and let the UI poll it.
//!
//! The worker publishes a progress text and watches a cancel flag; the UI asks `take` on a timer. A
//! panic in the worker becomes an error result instead of a job that never finishes.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// What the worker sees: a cancel flag it should honour and a place to say how far it got.
pub struct Ctl {
    cancel: AtomicBool,
    progress: Mutex<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Ctl {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// For APIs that take the flag itself (the scan).
    pub fn flag(&self) -> &AtomicBool {
        &self.cancel
    }

    pub fn set_progress(&self, text: String) {
        *lock(&self.progress) = text;
    }
}

pub struct Background<T> {
    ctl: Arc<Ctl>,
    result: Arc<Mutex<Option<Result<T, String>>>>,
}

fn panic_text(payload: Box<dyn Any + Send>) -> String {
    let detail = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_default();
    format!("something went wrong inside the app ({detail}); nothing else was changed")
}

impl<T: Send + 'static> Background<T> {
    pub fn spawn(work: impl FnOnce(&Ctl) -> T + Send + 'static) -> Background<T> {
        let ctl = Arc::new(Ctl { cancel: AtomicBool::new(false), progress: Mutex::new(String::new()) });
        let result = Arc::new(Mutex::new(None));
        {
            let (ctl, result) = (ctl.clone(), result.clone());
            std::thread::spawn(move || {
                let out = catch_unwind(AssertUnwindSafe(|| work(&ctl))).map_err(panic_text);
                *lock(&result) = Some(out);
            });
        }
        Background { ctl, result }
    }

    pub fn cancel(&self) {
        self.ctl.cancel.store(true, Ordering::Relaxed);
    }

    pub fn progress(&self) -> String {
        lock(&self.ctl.progress).clone()
    }

    /// The finished result, once. `None` while the worker is still running (or after it was taken).
    pub fn take(&self) -> Option<Result<T, String>> {
        lock(&self.result).take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait<T: Send + 'static>(job: &Background<T>) -> Result<T, String> {
        let start = Instant::now();
        loop {
            if let Some(r) = job.take() {
                return r;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "job never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_result_arrives_once() {
        let job = Background::spawn(|ctl| {
            ctl.set_progress("half way".into());
            7
        });
        assert_eq!(wait(&job), Ok(7));
        assert!(job.take().is_none());
    }

    #[test]
    fn cancel_reaches_the_worker() {
        let job = Background::spawn(|ctl| {
            while !ctl.cancelled() {
                std::thread::sleep(Duration::from_millis(2));
            }
            "stopped"
        });
        job.cancel();
        assert_eq!(wait(&job), Ok("stopped"));
    }

    #[test]
    fn a_panic_is_an_error_not_a_hang() {
        let job = Background::spawn(|_| -> u8 { panic!("boom") });
        let err = wait(&job).unwrap_err();
        assert!(err.contains("boom"), "{err}");
    }
}
