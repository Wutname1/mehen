//! Stopping work that is no longer wanted.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

/// A shared stop switch. Clones see the same switch.
#[derive(Clone)]
pub struct Cancel(Arc<watch::Sender<bool>>);

impl Default for Cancel {
    fn default() -> Self {
        Cancel(Arc::new(watch::channel(false).0))
    }
}

impl Cancel {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Resolves once the switch is thrown; never, otherwise.
    pub async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        let _ = rx.wait_for(|stop| *stop).await;
    }
}

/// One switch per repository in a batch, and a way to throw all of them.
#[derive(Default)]
pub struct Cancels {
    all: Cancel,
    jobs: Mutex<HashMap<String, Cancel>>,
}

impl Cancels {
    /// The switch for one job, made on first use. After "cancel all", new
    /// switches start thrown.
    pub fn job(&self, key: &str) -> Cancel {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.entry(key.to_lowercase())
            .or_insert_with(|| {
                let switch = Cancel::default();
                if self.all.is_cancelled() {
                    switch.cancel();
                }
                switch
            })
            .clone()
    }

    /// Cancels one job, or every job when `key` is `None`.
    pub fn cancel(&self, key: Option<&str>) {
        match key {
            Some(key) => self.job(key).cancel(),
            None => {
                self.all.cancel();
                for switch in self.jobs.lock().unwrap_or_else(|e| e.into_inner()).values() {
                    switch.cancel();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_job_or_all_of_them() {
        let cancels = Cancels::default();
        let (a, b) = (cancels.job("C:/code/A"), cancels.job("C:/code/B"));
        cancels.cancel(Some("c:/code/a"));
        assert!(a.is_cancelled() && !b.is_cancelled(), "the key is matched without case");
        a.cancelled().await;
        cancels.cancel(None);
        assert!(b.is_cancelled() && cancels.job("C:/code/C").is_cancelled(), "all of them, including ones not started yet");
    }
}
