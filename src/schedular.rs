use std::{collections::{HashSet, VecDeque}, sync::Arc};
use tokio::sync::{Mutex, Notify, mpsc};

/// Represents lifecycle changes for a resource.
/// These events are typically converted into keys for the WorkQueue.
#[derive(Debug, Clone)]
pub enum Event {
    Put(String),
    Delete(String),
}

/// A wrapper around a channel sender to broadcast resource events.
/// Acts as the bridge between the Store/Watcher and the Scheduler.
#[derive(Clone)]
pub struct Watcher {
    tx: mpsc::Sender<Event>,
}

impl Watcher {
    pub fn new(tx: mpsc::Sender<Event>) -> Self {
        Self { tx }
    }

    /// Dispatches an event to the scheduler asynchronously.
    pub async fn emit(&self, event: Event) {
        let _ = self.tx.send(event).await;
    }
}

/// A thread-safe, rate-limited work coordinator.
/// 
/// Uses a "dirty set" (HashSet) to ensure that a specific resource key 
/// is only queued once, even if multiple events are emitted.
#[derive(Clone)]
pub struct WorkQueue {
    inner: Arc<Inner>
}

/// Shared state for the WorkQueue.
pub struct Inner { 
    /// The ordered list of resource keys awaiting reconciliation.
    queue: Mutex<VecDeque<String>>,
    /// Tracks keys currently in the queue to prevent duplicate work.
    in_queue: Mutex<HashSet<String>>,
    /// Wakes up the worker loop when new work is pushed.
    notify: Notify
}

impl WorkQueue {
    /// Initializes the queue with shared state and notification primitives.
    pub fn new() -> Self {
        let inner = Inner { 
            queue: Mutex::new(VecDeque::new()),
            in_queue: Mutex::new(HashSet::new()),
            notify: Notify::new()
        };
        Self { 
            inner: Arc::new(inner)
        }
    }

    /// Adds a key to the queue if it isn't already present.
    /// Notifies a waiting worker via the internal Notify primitive.
    pub async fn push(&self, key: String) {
        let mut in_queue = self.inner.in_queue.lock().await;
        if in_queue.contains(&key) { 
            return; // Key is already scheduled for processing
        }
        
        let mut queue = self.inner.queue.lock().await;
        in_queue.insert(key.clone());
        queue.push_back(key.clone());
        self.inner.notify.notify_one();
    }

    /// Pulls the next available key from the queue.
    /// If the queue is empty, it park the current task until notified.
    pub async fn pop(&self) -> String {
        loop { 
            if let Some(key) = self.inner.queue.lock().await.pop_front() { 
                self.inner.in_queue.lock().await.remove(&key);
                return key;
            }
            // Wait for push() to signal new data
            self.inner.notify.notified().await;
        }
    } 
}