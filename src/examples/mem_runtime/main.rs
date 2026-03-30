use std::{collections::HashMap, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use rux_controller::{controller::{Context, Controller, Runtime}, resource::Resource, schedular::WorkQueue, kv_store};



#[derive(Clone, Serialize, Deserialize)]
pub struct DeploymentSpec { 
    replicas : u32
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct DeploymentStatus { 
    ready_replicas: u32
}

#[derive(Clone)]
pub struct Deployment { 
    pub name: String,
    pub spec: DeploymentSpec, 
    pub status: DeploymentStatus
}

impl Resource for Deployment {
    type Spec = DeploymentSpec;

    type Status = DeploymentStatus;

    fn key(&self) -> String {
        return format!("deployment/{}", self.name)
    }
}


pub struct InMemoryRuntime { 
    pub state: Arc<RwLock<HashMap<String, Deployment>>>
}

impl InMemoryRuntime { 
    pub fn new() -> Self { 
        Self { state: Arc::new(RwLock::new(HashMap::new())) }
    }
}

#[async_trait::async_trait]
impl Runtime<Deployment> for InMemoryRuntime {
    async fn observe(&self, key: &str) -> Option<DeploymentStatus> {
        if let Some(deployment) = self.state.read().await.get(key) { 
            return Some(deployment.clone().status);
        }
        None
    }

    async fn apply(&self, key: &str, desired: &DeploymentSpec) {
        // get the actual state
        if let Some(deployment) = self.state.write().await.get_mut(key) { 
            deployment.spec.replicas = desired.replicas;
        }
    }
    
}

pub struct DeploymentController;


impl Controller<Deployment> for DeploymentController {
    fn reconcile(
        &self,
        key: String,
        context: rux_controller::controller::Context<Deployment>
    ) -> impl Future<Output=std::io::Result<()>> {
        async  move { 
            let desired = match context.store.get(key.clone()).await { 
                Some(d) => d,
                None => return Ok(())
            };
            let actual = context.runtime.observe(&key).await;
            let actual_replicas = actual.map(|a| a.ready_replicas).unwrap_or(0);
            if actual_replicas != desired.spec.replicas { 
                println!("applying");
                context.runtime.apply(&key, &desired.spec).await;
                context.store.put(desired.key(), desired).await;
            }
            return Ok(());
        }
    }
}

#[tokio::main]
async fn main() {
    let runtime = Arc::new(InMemoryRuntime::new());
    let kv_store = kv_store::KVStore::new();
    let context = Context { store: kv_store.clone(), runtime };
    
    let (queue, mut rx) = WorkQueue::new(100);
    let controller = DeploymentController;
    // worker loop 
    tokio::spawn(async move { 
        while let Some(key) = rx.pop().await { 
            let _ = controller.reconcile(key.clone(), context.clone()).await;
        }
    });


    // create a resource
    let nginx_dep = Deployment { 
        name: "ngnix".to_string(),
        spec: DeploymentSpec { replicas: 4 },
        status: Default::default()
    };

    kv_store.put(nginx_dep.clone().key(), nginx_dep.clone()).await;
    queue.push(nginx_dep.key()).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
}
