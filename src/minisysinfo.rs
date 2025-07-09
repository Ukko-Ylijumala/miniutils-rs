// Copyright (c) 2025 Mikko Tanner. All rights reserved.

use crate::HumanBytes as HuB;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use timesince::TimeSinceEpoch;

const MIN_INTERVAL: Duration = Duration::from_millis(200);

/// Static information about the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SysInfoStatic {
    pub hostname: String,
    pub os_name: String,
    pub os_ver: String,
    pub kernel: String,
    pub distro: String,
    pub boot_time: u64,
    pub num_cores: usize,
}

/// SysInfoDynamic contains the mutable system information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SysInfoDynamic {
    pub mem: MemoryStats,
    pub cpu: f32,
    pub load: (f64, f64, f64),
    pub when: f64,
}

/// Memory information for the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total: u64,
    pub free: u64,
    pub avail: u64,
    pub buffers: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

/// Information about the system.
pub struct SysInfo {
    sys: RwLock<System>,
    kind: RefreshKind,
    inner: RwLock<SysInfoDynamic>,
    pub data: SysInfoStatic,
}

impl SysInfoStatic {
    /// Collect static system information from [sysinfo::System].
    fn collect() -> Self {
        Self {
            boot_time: System::boot_time(),
            num_cores: System::physical_core_count().unwrap_or(1),
            hostname: System::host_name().unwrap_or_else(|| "unknown".to_string()),
            os_name: System::name().unwrap_or_else(|| "unknown".to_string()),
            os_ver: System::os_version().unwrap_or_else(|| "unknown".to_string()),
            kernel: System::kernel_version().unwrap_or_else(|| "unknown".to_string()),
            distro: System::distribution_id(),
        }
    }
}

impl MemoryStats {
    /// Collect (initial) memory information from [sysinfo::System].
    fn collect(sys: &System) -> Self {
        Self {
            total: sys.total_memory(),
            free: sys.free_memory(),
            avail: sys.available_memory(),
            swap_total: sys.total_swap(),
            swap_used: sys.used_swap(),
            buffers: 0,
            cached: 0,
        }
    }

    /// Update memory information from [sysinfo::System].
    fn update(&mut self, sys: &System) {
        self.total = sys.total_memory();
        self.free = sys.free_memory();
        self.avail = sys.available_memory();
        self.swap_total = sys.total_swap();
        self.swap_used = sys.used_swap();
        // Buffers and cached are not available in sysinfo, so we set them to 0
        self.buffers = 0;
        self.cached = 0;
    }
}

impl SysInfo {
    pub fn new() -> Self {
        // Create a System object to query system information
        let kind = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything().without_frequency())
            .with_memory(MemoryRefreshKind::everything());
        let mut sys = System::new_with_specifics(kind);
        std::thread::sleep(MIN_INTERVAL); // must sleep for accurate initial data
        sys.refresh_specifics(kind);

        Self {
            kind,
            inner: RwLock::new(SysInfoDynamic {
                mem: MemoryStats::collect(&sys),
                cpu: sys.global_cpu_usage(),
                load: get_load_avg(),
                when: TimeSinceEpoch::now(),
            }),
            sys: RwLock::new(sys),
            data: SysInfoStatic::collect(),
        }
    }

    /// Refresh the inner system info struct (at most, once every [MIN_INTERVAL] ms)
    fn refresh(&self) {
        if TimeSinceEpoch::new_from(self.inner.read().when).since() < MIN_INTERVAL {
            return;
        }
        let mut sys = self.sys.write();
        let mut i = self.inner.write();
        sys.refresh_specifics(self.kind);
        i.when = TimeSinceEpoch::now();
        i.mem.update(&sys);
        i.cpu = sys.global_cpu_usage();
        i.load = get_load_avg();
    }

    /// Total memory in bytes.
    ///
    /// NOTE: system info is updated when calling this method.
    pub fn mem(&self) -> u64 {
        self.refresh();
        self.inner.read().mem.total
    }

    /// Used memory in bytes.
    ///
    /// NOTE: system info is updated when calling this method.
    pub fn mem_used(&self) -> u64 {
        self.refresh();
        let i = self.inner.read();
        i.mem.total - i.mem.avail
    }

    /// Available memory in bytes.
    ///
    /// NOTE: system info is updated when calling this method.
    pub fn mem_avail(&self) -> u64 {
        self.refresh();
        self.inner.read().mem.avail
    }

    /// Free memory in bytes.
    ///
    /// NOTE: system info is updated when calling this method.
    pub fn mem_free(&self) -> u64 {
        self.refresh();
        self.inner.read().mem.free
    }

    /// CPU usage as a percentage.
    ///
    /// NOTE: system info is updated when calling this method.
    pub fn cpu(&self) -> f32 {
        self.refresh();
        self.inner.read().cpu
    }

    /// Load averages for the last 1, 5, and 15 minutes.
    pub fn load(&self) -> (f64, f64, f64) {
        self.refresh();
        self.inner.read().load
    }

    /// Total memory in human-readable format, f.ex. "1.2 GiB".
    pub fn mem_str(&self) -> String {
        num2human(self.mem())
    }
    /// Available memory in human-readable format, f.ex. "1.2 GiB".
    pub fn mem_avail_str(&self) -> String {
        num2human(self.mem_avail())
    }
    /// Used memory in human-readable format, f.ex. "1.2 GiB".
    pub fn mem_used_str(&self) -> String {
        num2human(self.mem_used())
    }
    /// Free memory in human-readable format, f.ex. "1.2 GiB".
    pub fn mem_free_str(&self) -> String {
        num2human(self.mem_free())
    }

    /// CPU usage in human-readable format, f.ex. "10.25%".
    pub fn cpu_str(&self) -> String {
        format!("{:.2}%", self.cpu())
    }

    /// Print the system information to stderr.
    ///
    /// Format: `<ts> mem: 1 GiB used: 200 MiB avail: 800 MiB CPU: 5.55% load: 0.30 0.20 0.10`
    pub fn print(&self) {
        self.refresh();
        let i = self.inner.read();
        let (mem, avail) = (i.mem.total, i.mem.avail);
        let used = mem - avail;
        let cpu = i.cpu;
        let (l1, l5, l15) = i.load;
        let ts = TimeSinceEpoch::new_from(i.when);
        drop(i); // release the read lock before printing
        eprintln!(
            "{} | mem: {} used: {} avail: {} CPU: {:.2}% load: {:.2} {:.2} {:.2}",
            ts,
            num2human(mem),
            num2human(used),
            num2human(avail),
            cpu,
            l1,
            l5,
            l15
        );
    }
}

// ######## UTILITY FUNCTIONS ########

/// Convert a large (base-2) number to a human-readable format.
#[inline]
fn num2human(num: u64) -> String {
    HuB::to_human(num as f64, false, 2).unwrap_or("0.0".to_string())
}

/// Get the system load averages.
fn get_load_avg() -> (f64, f64, f64) {
    let load = System::load_average();
    (load.one, load.five, load.fifteen)
}
