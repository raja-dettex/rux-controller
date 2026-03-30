use tokio::sync::mpsc;

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
    tx: mpsc::Sender<String>,
}

pub struct WorkQueueReceiver {
    rx: mpsc::Receiver<String>,
}

impl WorkQueue {
    pub fn new(buffer: usize) -> (Self, WorkQueueReceiver) {
        let (tx, rx) = mpsc::channel(buffer);
        (Self { tx }, WorkQueueReceiver { rx })
    }

    pub async fn push(&self, key: String) {
        let _ = self.tx.send(key).await;
    }
}

impl WorkQueueReceiver {
    pub async fn pop(&mut self) -> Option<String> {
        self.rx.recv().await
    }
}