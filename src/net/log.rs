//! Small diagnostic log shared by host and join networking code.
//!
//! Network logs intentionally use the same elapsed-time shape as the local
//! session debug log so race reports are easy to compare across modes.

use std::{
    collections::VecDeque,
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{Context, Result};

pub type SharedNetworkLog = Arc<Mutex<NetworkLog>>;

#[derive(Debug, Clone)]
pub struct NetworkLog {
    started_at: Instant,
    entries: VecDeque<String>,
    capacity: usize,
}

impl NetworkLog {
    pub fn new(started_at: Instant, capacity: usize) -> Self {
        Self {
            started_at,
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn shared(started_at: Instant, capacity: usize) -> SharedNetworkLog {
        Arc::new(Mutex::new(Self::new(started_at, capacity)))
    }

    pub fn push(&mut self, now: Instant, message: impl Into<String>) {
        if self.capacity == 0 {
            return;
        }

        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }

        let elapsed_ms = now.saturating_duration_since(self.started_at).as_millis();
        self.entries
            .push_back(format!("+{elapsed_ms:>6}ms {}", message.into()));
    }

    pub fn entries(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(String::as_str)
    }
}

pub fn push_network_log(log: &Option<SharedNetworkLog>, message: impl Into<String>) {
    let Some(log) = log else {
        return;
    };

    if let Ok(mut log) = log.lock() {
        log.push(Instant::now(), message);
    }
}

pub fn write_network_log(path: impl AsRef<Path>, log: &SharedNetworkLog) -> Result<()> {
    let path = path.as_ref();
    let contents = log
        .lock()
        .expect("network log poisoned")
        .entries()
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(path, format!("{contents}\n"))
        .with_context(|| format!("failed to write network debug log to {}", path.display()))
}

#[cfg(test)]
mod tests;
