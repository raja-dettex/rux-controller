use std::sync::Arc;
use crate::{kv_store::KVStore, resource::Resource};

/// Defines the interaction with the external environment.
/// 
/// The Runtime is responsible for fetching real-world state (observe)
/// and executing changes to reach the desired state (apply).
#[async_trait::async_trait]
pub trait Runtime<T: Resource>: Send + Sync {  
    /// Fetches the current status of a specific resource from the infrastructure.
    async fn observe(&self, key: &str) -> Option<T::Status>;
    
    /// Executes the necessary actions to align the infrastructure with the Spec.
    async fn apply(&self, key: &str, desired: &T::Spec);
} 

/// Shared dependencies provided to every reconciliation attempt.
/// 
/// Bundles the local cache (Store) and the effect-handler (Runtime) 
/// into a single thread-safe container.
#[derive(Clone)]
pub struct Context<T> { 
    /// Local cache of resources for quick lookups.
    pub store: KVStore<T>,
    /// The engine that performs the actual infrastructure mutations.
    pub runtime: Arc<dyn Runtime<T>>
}

/// The core logic provider for a specific Resource type.
/// 
/// Implementations of this trait contain the business logic required to
/// determine the delta between Spec and Status.
pub trait Controller<T>: Send + Sync
where T: Send + Sync
{ 
    /// The entry point for the reconciliation cycle.
    /// 
    /// Triggered by the WorkQueue, this method receives a resource key 
    /// and uses the Context to drive the resource toward its desired state.
    fn reconcile(
        &self,
        key: String,
        context: Context<T>
    ) -> impl Future<Output=std::io::Result<()>>;
}