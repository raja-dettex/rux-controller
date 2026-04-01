use std::{collections::{HashSet, VecDeque}, sync::Arc};

use tokio::sync::{Mutex, Notify, mpsc};

#[derive(Debug, Clone)]
pub enum Event {
    Put(String),
    Delete(String),
}

#[derive(Clone)]
pub struct Watcher {
    tx: mpsc::Sender<Event>,
}

impl Watcher {
    pub fn new(tx: mpsc::Sender<Event>) -> Self {
        Self { tx }
    }

    pub async fn emit(&self, event: Event) {
        let _ = self.tx.send(event).await;
    }
}


#[derive(Clone)]
pub struct WorkQueue {
    inner: Arc<Inner>
}

pub struct Inner { 
    queue: Mutex<VecDeque<String>>,
    in_queue: Mutex<HashSet<String>>,
    notify: Notify
}

pub struct WorkQueueReceiver {
    rx: mpsc::Receiver<String>,
}

impl WorkQueue {
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

    pub async fn push(&self, key: String) {
        println!("pushing ");
        let mut in_queue = self.inner.in_queue.lock().await;
        if in_queue.contains(&key) { 
            println!("notified but duplicates");
            return
        }
        
        let mut queue = self.inner.queue.lock().await;
        in_queue.insert(key.clone());
        queue.push_back(key.clone());
        println!("not duplictest");
        self.inner.notify.notify_one();
    }

    pub async fn pop(&self) -> String {
        loop { 
            if let Some(key) = self.inner.queue.lock().await.pop_front() { 
                println!("recieved key {key}");
                self.inner.in_queue.lock().await.remove(&key);
                return key;
            }
            self.inner.notify.notified().await;
        }
    } 
}

impl WorkQueueReceiver {
    pub async fn pop(&mut self) -> Option<String> {
        self.rx.recv().await
    }
}