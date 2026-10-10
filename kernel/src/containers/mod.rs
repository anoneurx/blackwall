/// Containers — Namespace isolation for PID, Mount, and Network namespaces
extern crate alloc;

use crate::net::{IpAddress, MacAddress};
use crate::sync::spin::SpinLock;
use alloc::string::String;
use alloc::vec::Vec;

// ── Namespace types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PidNamespace {
    pub id: u64,
    /// PIDs visible within this namespace (mapped from global PIDs)
    pub pid_map: Vec<(u64, u64)>, // (global_pid, ns_pid)
    pub next_ns_pid: u64,
}

impl PidNamespace {
    pub fn new(id: u64) -> Self {
        Self { id, pid_map: Vec::new(), next_ns_pid: 1 }
    }

    pub fn allocate(&mut self, global_pid: u64) -> u64 {
        let ns_pid = self.next_ns_pid;
        self.next_ns_pid += 1;
        self.pid_map.push((global_pid, ns_pid));
        ns_pid
    }

    pub fn resolve(&self, global_pid: u64) -> Option<u64> {
        self.pid_map.iter().find(|(gp, _)| *gp == global_pid).map(|(_, np)| *np)
    }
}

#[derive(Debug, Clone)]
pub struct MountNamespace {
    pub id: u64,
    /// List of (host_path, container_path) bind mounts
    pub mounts: Vec<(String, String)>,
}

impl MountNamespace {
    pub fn new(id: u64) -> Self {
        Self { id, mounts: Vec::new() }
    }

    pub fn bind(&mut self, host: &str, container: &str) {
        self.mounts.push((String::from(host), String::from(container)));
    }
}

#[derive(Debug, Clone)]
pub struct NetNamespace {
    pub id: u64,
    pub ip: IpAddress,
    pub mac: MacAddress,
}

impl NetNamespace {
    pub fn new(id: u64, ip: IpAddress, mac: MacAddress) -> Self {
        Self { id, ip, mac }
    }
}

// ── Container ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Container {
    pub id: u64,
    pub name: String,
    pub pid_ns: PidNamespace,
    pub mnt_ns: MountNamespace,
    pub net_ns: NetNamespace,
}

impl Container {
    pub fn new(id: u64, name: &str) -> Self {
        let net_ns = NetNamespace::new(
            id,
            IpAddress([172, 17, id as u8, 2]),
            MacAddress([0x02, 0x00, 0x00, 0x00, 0x00, id as u8]),
        );
        Self {
            id,
            name: String::from(name),
            pid_ns: PidNamespace::new(id),
            mnt_ns: MountNamespace::new(id),
            net_ns,
        }
    }
}

// ── Container manager ────────────────────────────────────────────────────────

pub static CONTAINER_MANAGER: SpinLock<ContainerManager> =
    SpinLock::new(ContainerManager { containers: Vec::new(), next_id: 1 });

pub struct ContainerManager {
    pub containers: Vec<Container>,
    pub next_id: u64,
}

impl ContainerManager {
    pub fn create(&mut self, name: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.containers.push(Container::new(id, name));
        id
    }

    pub fn get(&self, id: u64) -> Option<&Container> {
        self.containers.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Container> {
        self.containers.iter_mut().find(|c| c.id == id)
    }

    pub fn destroy(&mut self, id: u64) {
        self.containers.retain(|c| c.id != id);
    }
}

pub fn init() {
    use crate::arch::x86_64::serial;
    serial::line("[CONTAINERS] Initializing container runtime...");

    // Create the root system container (host namespace)
    let id = CONTAINER_MANAGER.lock().create("host");
    serial::line(&alloc::format!("[CONTAINERS] Root 'host' container created (id={id})."));
    serial::line("[CONTAINERS] Container runtime ready.");
}
