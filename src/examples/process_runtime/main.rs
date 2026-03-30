use std::{collections::HashMap, process::Stdio, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::{process::{Child, Command}, sync::RwLock};

use rux_controller::{controller::{Context, Controller, Runtime}, kv_store, resource::Resource, schedular::{Watcher, WorkQueue}};



#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct DeploymentSpec { 
    replicas : u32
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct DeploymentStatus { 
    ready_replicas: u32
}

#[derive(Clone, Debug)]
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


pub struct ProcessRuntime { 
    pub state: Arc<RwLock<HashMap<String, Vec<Child>>>>
}

impl ProcessRuntime { 
    pub fn new() -> Self { 
        Self { state: Arc::new(RwLock::new(HashMap::new())) }
    }
}


#[async_trait::async_trait]
impl Runtime<Deployment> for ProcessRuntime { 
    async fn observe(&self, key: &str) -> Option<DeploymentStatus> { 
        if let Some(processes) = self.state.write().await.get_mut(key) { 
            processes.retain_mut(|child| match child.try_wait() { 
                Ok(Some(_)) => false,
                _ => true
            });
            return Some(DeploymentStatus { ready_replicas: processes.len() as u32});
        } else { 
            return Some(DeploymentStatus {ready_replicas: 0 });
        }
    }

    async fn apply(&self, key: &str, desired: &DeploymentSpec) { 
        let mut guard =  self.state.write().await;
        let actual = guard.entry(key.to_string()).or_default();
        let current = actual.len();
        if desired.replicas as usize > current { 
            let to_spawn = desired.replicas as usize - current;
            for _ in 0..to_spawn {
                let child = Command::new("sleep")
                    .arg("1000")
                    .stdout(Stdio::null())
                    .spawn()
                    .expect("failed to spawn");

                actual.push(child);
            }
        }
    }
}

pub struct ProcessController;

impl Controller<Deployment> for ProcessController {
    fn reconcile(
        &self,
        key: String,
        context: Context<Deployment>
    ) -> impl Future<Output=std::io::Result<()>> {
        async move { 
            if let Some(actual) = context.runtime.observe(&key).await { 
                let current_replicas = actual.ready_replicas;
                let desired = match context.store.get(key.clone()).await { 
                    Some(d) => d,
                    None => return Ok(())
                };
                let desired_spec = desired.spec;
                if current_replicas != desired_spec.replicas { 
                     println!(
                        "[RECONCILE] {}: actual={:?} desired={:?}",
                        key, actual, desired_spec
                    );
                    context.runtime.apply(&key, &desired_spec).await;
                }
            }

            return Ok(());
        }
    }
}

#[tokio::main]
async fn main() { 
    let runtime = Arc::new(ProcessRuntime::new());
    let kv_store = kv_store::KVStore::new();
    let context = Context { store: kv_store.clone(), runtime };
    
    let (queue, mut rx) = WorkQueue::new(100);
    let controller = ProcessController;


    // worker loop 
    tokio::spawn(async move { 
        while let Some(key) = rx.pop().await { 
            println!("received");
            let _ = controller.reconcile(key.clone(), context.clone()).await;
        }
    });


    // create a resource
    let nginx_dep = Deployment { 
        name: "ngnix".to_string(),
        spec: DeploymentSpec { replicas: 4 },
        status: Default::default()
    };

    let patched_deployment = Deployment { 
        name: "ngnix".to_string(),
        spec: DeploymentSpec { replicas: 6 },
        status: Default::default()
    };

    kv_store.put(nginx_dep.clone().key(), nginx_dep.clone()).await;
    queue.push(nginx_dep.key()).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    kv_store.put(patched_deployment.clone().key() , patched_deployment.clone()).await;
    queue.push(patched_deployment.key()).await;
    tokio::signal::ctrl_c().await;
}