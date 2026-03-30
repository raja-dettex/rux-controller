use std::sync::Arc;

use crate::{kv_store::KVStore, resource::Resource};

#[async_trait::async_trait]
pub trait Runtime<T: Resource>: Send + Sync {  
    async fn observe(&self, key: &str) -> Option<T::Status>;
    async fn apply(&self, key: &str, desired: &T::Spec);
} 


#[derive(Clone)]
pub struct Context<T> { 
    pub store: KVStore<T>,
    pub runtime: Arc<dyn Runtime<T>>
}


pub trait Controller<T>: Send + Sync
where T: Send + Sync
{ 
    fn reconcile(
        &self,
        key: String,
        context: Context<T>
    ) -> impl Future<Output=std::io::Result<()>>;
}

