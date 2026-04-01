use std::{collections::HashMap, sync::{Arc, }};

use tokio::sync::RwLock;


#[derive(Clone)]
pub struct KVStore<T> { 
    inner: Arc<RwLock<HashMap<String, T>>>    
}

impl<T: Clone> KVStore<T> { 
    pub fn new() -> Self { 
        KVStore { inner: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn put(&self, name: String, t: T) { 
        self.inner.write().await.insert(name, t);
    }

    pub async fn get(&self, name: String) -> Option<T> { 
        self.inner.read().await.get(&name).cloned()
    }

    #[inline]
    pub async fn list_keys(&self) -> Vec<String> { 
        self.inner.read().await.keys().map(|k| k.clone()).collect()
    }
}


