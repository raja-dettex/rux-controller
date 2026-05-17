use std::{
    collections::{HashMap, HashSet, hash_map::Entry}, io::ErrorKind, str::FromStr, sync::Arc, thread::current, time::Duration
};

use bollard::{Docker, plugin::{ContainerCreateBody, ContainerInspectResponse, HostConfig, NetworkCreateRequest, PortBinding}, query_parameters::{CreateContainerOptions, CreateContainerOptionsBuilder, ListContainersOptions, ListContainersOptionsBuilder, StartContainerOptions}};
use rand::seq::IteratorRandom;
use rux_controller::{controller::{Context, Runtime}, kv_store::KVStore, resource::Resource, schedular::WorkQueue};
use serde::de;
use tokio::{process::Command, sync::RwLock};

use async_trait::async_trait;


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

#[derive(PartialEq, Eq, Clone)]
pub enum NetworkInterface { 
    Bridge,
    Host
}

impl FromStr for NetworkInterface {
    type Err = std::io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s { 
            "bridge" => Ok(Self::Bridge),
            "host" => Ok(Self::Host),
            _ => Err(std::io::Error::new(ErrorKind::Other, "unknown type, can not parse it out"))
        }
    }
}

impl Into<String> for NetworkInterface { 
    fn into(self) -> String {
        match self { 
            Self::Bridge => "bridge".to_string(),
            Self::Host => "host".to_string()
        }
    }    
}
pub struct NetworkConfig { 
    namespace_name: String,
    interface_type: NetworkInterface        
}

#[derive(Clone)]
pub struct Builder { 
    daemon: Docker,
    networks: Vec<(String, NetworkInterface)>,
}

use bollard::models::ContainerSummary;

impl Builder { 
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let docker = Docker::connect_with_defaults()?;
        Ok(Self {
            daemon: docker,
            networks: Vec::new()
        })
    }

    pub async fn init_builder_context(
        &self,
        network_config: NetworkConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.networks.contains(&(
            network_config.namespace_name.clone(),
            network_config.interface_type.clone(),
        )) {
            return Ok(());
        }
        let net_name = network_config.namespace_name.clone();
        let driver: String = network_config.interface_type.into();
        let net_config = NetworkCreateRequest {
            name: net_name,
            driver: Some(driver),
            ..Default::default()
        };
        let _ = self.create_network(net_config).await;
        Ok(())
    }

    pub async fn create_network(&self, net_conf: NetworkCreateRequest) {
        let _ = self.daemon.create_network(net_conf).await;
    }

    pub async fn creat_and_start_container(
        &self,
        name: String,
        create_container_options: Option<CreateContainerOptions>,
        container_conf: ContainerCreateBody,
        start_container_options: Option<StartContainerOptions>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.daemon
            .create_container(create_container_options, container_conf)
            .await?;
        self.daemon
            .start_container(&name, start_container_options)
            .await?;
        Ok(())
    }

    pub async fn list_containers(&self, filters: HashMap<String, Vec<String>>) -> Option<Vec<String>>{ 
        let list_options = ListContainersOptionsBuilder::default().all(true).filters(&filters).build();  
        let ids = if let Ok(res) = self.daemon.list_containers(Some(list_options)).await { 
            Some(res.into_iter().filter_map(|c| c.id).collect())
        } else { 
            None
        };
        ids
    }

    pub async fn inspect_containers(&self, container_ids: Vec<String>) -> Vec<ContainerInspectResponse> { 
        // Assuming `container_ids` is the Vec<String> from the previous step
        let mut results = Vec::new();

        for id in container_ids {
            if let Ok(inspect_result) = self.daemon.inspect_container(&id, None).await {
                // Access .config then .labels (both are Option types in Bollard)
                results.push(inspect_result);                
                
            }
        }
        results
    }
} 

#[derive(Clone)]
pub struct ContainerRuntime {
    pub state: Arc<RwLock<HashMap<String, DeploymentStatus>>>, // key -> container IDs
    builder: Builder
}

impl ContainerRuntime {

    pub fn new() -> Result<Self, Box<dyn std::error::Error>> { 
        Ok(Self { 
            state: Arc::new(RwLock::new(HashMap::new())),
            builder: Builder::new()?         
        })
    }

    pub async fn target_ports(&self, key: &str) -> Vec<i32> { 
        if let Some(status) = self.state.read().await.get(key) { 
            return status.replicas.clone().
            into_iter().map(|replica| replica.nats).clone().filter_map(|nat| {
                for (k,_) in nat { 
                    return Some(k);
                }
                None
            }).collect();
        }
        vec![]
    }
    pub async fn get_actual_set(&self, key: &str) -> HashSet<u32> {
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![format!("rux.key={}", key)]);
        let mut actual = HashSet::new();
        if let Some(ids) = self.builder.list_containers(filters).await { 
            let inspect_results = self.builder.inspect_containers(ids).await;
            for result in inspect_results { 
                if let Some(labels) = result.config.and_then(|c| c.labels) {
                    // Look for your specific label key
                    if let Some(replica_str) = labels.get("rux.replica") {
                        if let Ok(idx) = replica_str.trim().parse::<u32>() {
                            actual.insert(idx);
                        }
                    }
                }
            }
        }
        actual
        /* let mut actual = HashSet::new();
        
        

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

        actual*/
    }
}


#[async_trait]
impl Runtime<Deployment> for ContainerRuntime {
    async fn observe(&self, key: &str) -> Option<DeploymentStatus> {
        let mut guard = self.state.write().await;

        let status = guard.entry(key.to_string()).or_default();

        let mut alive = vec![];
        let container_ids = status.replicas.clone().into_iter().map(|r| r.id).collect();
        let container_ids_to_replica : HashMap<String, Pod> =  status.replicas.clone().into_iter()
            .map(|p| (p.id.clone(), p)).collect();
        let inspect_results = self.builder.inspect_containers(container_ids).await;
        
        for result in inspect_results { 
            if let Some(id) = result.id { 
                let running = result.state.and_then(|s| s.running).unwrap_or(false);
                if running { 
                    if let Some(replica) = container_ids_to_replica.get(&id) { 
                        alive.push(replica.clone());
                    }
                }
            }
        }
        let current_status = DeploymentStatus { ready_replicas: alive.len() as u32, replicas: alive};
        *status = current_status.clone();
        Some(current_status) 
        /* for replica in status.replicas.iter() {
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
        Some(current_status) */
    }

    async fn apply(&self, key: &str, desired: &DeploymentSpec) {
        let existing_target_ports: Vec<i32> = self.target_ports(key).await;
        let mut guard = self.state.write().await;
        let current_status = guard.entry(key.to_string()).or_default();
        let current = current_status.ready_replicas;
        let desired_replicas = desired.replicas;
        let desired_set: HashSet<u32> = (0..desired_replicas).collect();
        // SCALE UP
        if current < desired_replicas {
            
            
            let actual_set = self.get_actual_set(key).await;
            let to_spawn = &desired_set - &actual_set;

            for desired_value in to_spawn {
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
                let port_bindings = pod.nats.clone().into_iter().map(|(k, v)| { 
                    let key = format!("{}/tcp", k);
                    let val = Some(vec![PortBinding{
                        host_ip: Some("0.0.0.0".to_string()),
                        host_port: Some(format!("{v}"))
                    }]);
                    (key, val)
                }).collect();
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
                let binds = pod.volumes.clone().into_iter().map(|(h, c)| format!("{}:{}", h.clone(), c.clone())).collect();
                let mut labels = HashMap::new();
                labels.insert("rux.key".to_string(), key.to_string());
                labels.insert("rux.replica".to_string(), format!("{}", desired_value));
                let container_config = ContainerCreateBody { 
                    image: Some(desired.template.image_name.clone()),
                    labels: Some(labels),
                    host_config: Some(HostConfig {
                        port_bindings: Some(port_bindings),
                        binds: Some(binds),
                        network_mode: Some("bridge".to_string()),
                        privileged: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                let create_container_options = CreateContainerOptionsBuilder::default().name(&desired_name).build();
                let _ = self.builder.creat_and_start_container(
                    desired_name.clone(), 
                    Some(create_container_options), 
                    container_config, 
                    None
                ).await;
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
            let actual_set = self.get_actual_set(key).await;
            let to_kill = &actual_set - &desired_set;


            for i in to_kill {
                let name = format!("{}-{}",key, i);
                if let Some(id) = current_status.clone().replicas.iter().filter(|&pod| pod.name.eq(&name)).map(|pod| pod.id.clone()).next() {
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
async fn main() -> Result<(), Box<dyn std::error::Error>>{
    let store = KVStore::new();

    let runtime = Arc::new(ContainerRuntime::new()?);

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
    Ok(())
}