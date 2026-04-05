
/// Defines the core requirements for a system-managed object.
/// 
/// This trait ensures that any entity managed by the reconciliation engine
/// has a deterministic identity and verifiable state types.
pub trait Resource: Send + Sync + 'static {
    /// The desired state configuration (The "What").
    type Spec: Send + Sync + Clone;
    
    /// The current observed state of the infrastructure (The "Is").
    type Status: Send + Sync + Clone;

    /// Returns a unique identifier for the resource instance.
    /// Used by the WorkQueue and Indexer to prevent duplicate processing.
    fn key(&self) -> String;
}