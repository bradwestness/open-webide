//! Read-only snapshot of the machine running the execution bridge.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HostInfo {
    pub host_name: Option<String>,
    pub os: Option<String>,
    pub scope: String,
    pub cpu: Option<String>,
    pub logical_cores: usize,
    pub ram_total_bytes: u64,
    pub ram_available_bytes: u64,
    pub disks: Vec<HostDisk>,
    pub temperatures: Vec<HostTemperature>,
    pub gpus: Vec<HostGpu>,
    pub fans: Vec<HostFan>,
    pub notes: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostDisk {
    pub name: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostTemperature {
    pub name: String,
    pub celsius: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostGpu {
    pub name: String,
    pub vram_total_bytes: Option<u64>,
    pub vram_used_bytes: Option<u64>,
    pub temperature_celsius: Option<f32>,
    pub fan_percent: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostFan {
    pub name: String,
    pub rpm: u64,
}
