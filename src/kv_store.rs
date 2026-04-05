use std::{collections::HashMap, sync::{Arc, }};
use tokio::sync::RwLock;

/// A thread-safe, asynchronous Key-Value store.
/// 
/// Serves as the local cache (Informer Store) for resources, 
/// minimizing direct API/DB hits during the reconciliation loop.
#[derive(Clone)]
pub struct KVStore<T> { 
    inner: Arc<RwLock<HashMap<String, T>>>    
}

impl<T: Clone> KVStore<T> { 
    /// Initializes an empty store wrapped in an Arc for shared ownership.
    pub fn new() -> Self { 
        KVStore { inner: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// Inserts or updates a resource. 
    /// Acquires a write lock to ensure atomicity during the update.
    pub async fn put(&self, name: String, t: T) { 
        self.inner.write().await.insert(name, t);
    }

    /// Retrieves a cloned copy of a resource by name.
    /// Uses a read lock to allow multiple simultaneous readers.
    pub async fn get(&self, name: String) -> Option<T> { 
        self.inner.read().await.get(&name).cloned()
    }

    /// Returns a snapshot of all keys currently in the store.
    /// Useful for the "List" part of the List-Watch pattern.
    #[inline]
    pub async fn list_keys(&self) -> Vec<String> { 
        self.inner.read().await.keys().map(|k| k.clone()).collect()
    }
}