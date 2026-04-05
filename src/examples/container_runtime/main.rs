use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use rand::seq::IteratorRandom;
use rux_controller::{controller::{Context, Runtime}, kv_store::KVStore, resource::Resource, schedular::WorkQueue};
use tokio::{process::Command, sync::RwLock};

use async_trait::async_trait;

//
// ================= RESOURCE =================
//

#[derive(Clone, Debug)]
pub struct DeploymentSpec {
    pub replicas: u32,
    pub template: Template,
}

#[derive(Clone, Debug)]
pub struct Template {
    pub image_name: String,
    pub environments: HashMap<String, String>,
    pub nats: Vec<i32>, // host -> container
    pub volumes: HashMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct DeploymentStatus {
    pub ready_replicas: u32,
    pub replicas: Vec<Pod>
}

#[derive(Clone, Debug, Default)]
pub struct Pod { 
    id: String,
    name: String,
    volumes: HashMap<String, String>,
    nats: HashMap<i32, i32>
}

#[derive(Clone)]
pub struct Deployment {
    pub name: String,
    pub spec: DeploymentSpec,
    pub status: DeploymentStatus,
}

impl Resource for Deployment {
    
    fn key(&self) -> String {
        self.name.clone()
    }
    
    type Spec = DeploymentSpec;
    
    type Status = DeploymentStatus;
}

#[derive(Clone)]
pub struct ContainerRuntime {
    pub state: Arc<RwLock<HashMap<String, DeploymentStatus>>>, // key -> container IDs
}

impl ContainerRuntime {
    pub async fn get_actual_set(&self, key: &str) -> HashSet<u32> {
        let mut actual = HashSet::new();

        let out = Command::new("docker")
            .arg("ps")
            .arg("--filter")
            .arg(format!("label=rux.key={}", key))
            .arg("--format")
            .arg("{{.ID}}")
            .output()
            .await;

        let output = match out {
            Ok(out) => out,
            Err(_) => return actual,
        };

        let stdout = String::from_utf8_lossy(&output.stdout);

        for id in stdout.lines() {
            let inspect = Command::new("docker")
                .arg("inspect")
                .arg("-f")
                .arg("{{ index .Config.Labels \"rux.replica\" }}")
                .arg(id)
                .output()
                .await;

            if let Ok(inspect_out) = inspect {
                let val = String::from_utf8_lossy(&inspect_out.stdout);

                if let Ok(idx) = val.trim().parse::<u32>() {
                    actual.insert(idx);
                }
            }
        }

        actual
    }
}


#[async_trait]
impl Runtime<Deployment> for ContainerRuntime {
    async fn observe(&self, key: &str) -> Option<DeploymentStatus> {
        let mut guard = self.state.write().await;

        let status = guard.entry(key.to_string()).or_default();

        let mut alive = vec![];

        for replica in status.replicas.iter() {
            let out = Command::new("docker")
                .arg("inspect")
                .arg("-f")
                .arg("{{.State.Running}}")
                .arg(replica.id.clone())
                .output()
                .await;

            if let Ok(out) = out {
                let s = String::from_utf8_lossy(&out.stdout);
                if s.trim() == "true" {
                    alive.push(replica.clone());
                }
            }
        }

        let current_status = DeploymentStatus { ready_replicas: alive.len() as u32, replicas: alive };
        *status = current_status.clone();
        Some(current_status)
    }

    async fn apply(&self, key: &str, desired: &DeploymentSpec) {
        let mut guard = self.state.write().await;
        let current_status = guard.entry(key.to_string()).or_default();

        let current = current_status.ready_replicas;
        let desired_replicas = desired.replicas;
        let existing_target_ports: Vec<i32> = Vec::new();
        // SCALE UP
        if current < desired_replicas {
            
            let to_spawn = desired_replicas - current;
            let actual_set = self.get_actual_set(key).await;
            let diff_set: HashSet<u32> = (0..desired_replicas).filter(|&i| !actual_set.contains(&(i as u32))).collect();

            for desired_value in diff_set {
                let mut pod = Pod::default();
                let desired_name = format!("{}-{}", key, desired_value);
                pod.name = desired_name.clone();
                let mut cmd = Command::new("docker");
                cmd.arg("run").arg("-d").arg("--name").arg(&desired_name);
                let nats = desired.template.nats.clone();
                for container_port in nats {
                    let mut rng = rand::rng();
                    if let Some(target_port) = (0..25000).
                    filter(|i| !existing_target_ports.contains(i)).choose(&mut rng) {
                        cmd.arg("-p").arg(format!("{}:{}", target_port, container_port));
                        pod.nats.insert(target_port, container_port);    
                    }
                }

                // env (FIXED)
                for (k, v) in desired.template.environments.iter() {
                    cmd.arg("-e").arg(format!("{}={}", k, v));
                }

                // volumes
                for (h, d) in desired.template.volumes.iter() {
                    let host_volume = format!("{}/{}", h, desired_name);
                    pod.volumes.insert(host_volume.clone(), d.to_string());
                    cmd.arg("-v").arg(format!("{}:{}", host_volume, d));
                }
                cmd.arg("--label").arg(format!("rux.key={}", key));
                cmd.arg("--label").arg(format!("rux.replica={}", desired_value));

                cmd.arg(&desired.template.image_name);

                match cmd.output().await {
                    Ok(out) if out.status.success() => {
                        let id = String::from_utf8_lossy(&out.stdout)
                            .trim()
                            .to_string();

                        println!("[SPAWNED] {}", id);
                        pod.id = id;
                        current_status.replicas.push(pod);
                    }
                    Err(e) => println!("spawn error: {:?}", e),
                    Ok(out) => println!("docker run failed {:?}", out),
                }
            }
        }

        // SCALE DOWN
        else if current > desired_replicas {
            let to_kill = current - desired_replicas;

            for _ in 0..to_kill {
                if let Some(id) = current_status.replicas.iter().map(|pod| pod.id.clone()).next() {
                    let _ = Command::new("docker")
                        .arg("rm")
                        .arg("-f")
                        .arg(&id)
                        .output()
                        .await;

                    println!("[KILLED] {}", id);
                }
            }
        }
    }
}

//
// ================= CONTROLLER =================
//

pub struct ContainerController;

impl ContainerController {
    pub async fn reconcile(&self, key: String, ctx: Context<Deployment>) {
        let desired = match ctx.store.get(key.clone()).await {
            Some(d) => d,
            None => return,
        };

        let actual = ctx.runtime.observe(&key).await.unwrap();

        if actual.ready_replicas != desired.spec.replicas {
            println!(
                "[RECONCILE] {} actual={} desired={}",
                key, actual.ready_replicas, desired.spec.replicas
            );

            ctx.runtime.apply(&key, &desired.spec).await;
        }
    }
}


#[tokio::main]
async fn main() {
    let store = KVStore::new();

    let runtime = Arc::new(ContainerRuntime {
        state: Arc::new(RwLock::new(HashMap::new())),
    });

    let ctx = Context {
        store: store.clone(),
        runtime,
    };

    let queue = WorkQueue::new();

    // workers
    for _ in 0..4 {
        let q = queue.clone();
        let ctx_clone = ctx.clone();
        let controller = ContainerController;

        tokio::spawn(async move {
            loop {
                let key = q.pop().await;
                controller.reconcile(key, ctx_clone.clone()).await;
            }
        });
    }

    // periodic resync
    let q = queue.clone();
    let s = store.clone();

    tokio::spawn(async move {
        loop {
            for key in s.list_keys().await {
                q.push(key).await;
            }

            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });


    let mut ports = vec![80];
    let template = Template {
        image_name: "httpd:2.4".to_string(),
        environments: HashMap::new(),
        nats: ports,
        volumes: HashMap::new(),
    };

    let dep = Deployment {
        name: "apache".to_string(),
        spec: DeploymentSpec {
            replicas: 2,
            template,
        },
        status: Default::default(),
    };

    store.put(dep.key(), dep.clone()).await;
    queue.push(dep.key()).await;

    tokio::signal::ctrl_c().await.unwrap();
}