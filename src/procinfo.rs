// Copyright (c) 2024-2025 Mikko Tanner. All rights reserved.

use crate::HumanBytes as HuB;
use parking_lot::RwLock;
use std::time::{Duration, Instant};
use sysinfo::{Pid, System};

/// ProcessInfoInner contains the actual (mutable) process information.
struct ProcessInfoInner {
    sys: System,
    mem: u64,
    cpu: f32,
    upd: Instant,
}

/// Information about the current process.
pub struct ProcessInfo {
    pub pid: usize,
    p: Pid,
    inner: RwLock<ProcessInfoInner>,
}

impl ProcessInfo {
    pub fn new() -> Self {
        // Get the current process ID
        let pid: usize = std::process::id() as usize;
        let s_p: Pid = Pid::from(pid);

        // Create a System object to query system information
        let mut sys: System = System::new();
        sys.refresh_process(s_p);
        Self {
            pid,
            p: s_p,
            inner: RwLock::new(ProcessInfoInner {
                mem: sys.process(s_p).map_or_else(|| 0, |p| p.memory()),
                cpu: sys.process(s_p).map_or_else(|| 0.0, |p| p.cpu_usage()),
                sys,
                upd: Instant::now(),
            }),
        }
    }

    /// Refresh the inner process info struct (at most, once every 200 ms)
    fn refresh(&self) {
        if self.inner.read().upd.elapsed() < Duration::from_millis(200) {
            return;
        }
        let mut i = self.inner.write();
        i.sys.refresh_process(self.p);
        i.mem = i.sys.process(self.p).map_or_else(|| 0, |p| p.memory());
        i.cpu = i.sys.process(self.p).map_or_else(|| 0.0, |p| p.cpu_usage());
        i.upd = Instant::now();
    }

    /// Memory usage in bytes.
    ///
    /// Note: process info is updated when calling this method.
    pub fn mem(&self) -> u64 {
        self.refresh();
        self.inner.read().mem
    }

    /// CPU usage as a percentage.
    ///
    /// Note: process info is updated when calling this method.
    pub fn cpu(&self) -> f32 {
        self.refresh();
        self.inner.read().cpu
    }

    /// Memory usage in human-readable format, f.ex. "1.2 GiB".
    pub fn mem_str(&self) -> String {
        HuB::to_human(self.mem() as f64, false, 2).unwrap_or("0.0".to_string())
    }

    /// CPU usage in human-readable format, f.ex. "10.25%".
    pub fn cpu_str(&self) -> String {
        format!("{:.2}%", self.cpu())
    }

    /// Print the process information to stderr.
    /// Format: "pid: 123 mem: 10 MiB CPU: 5.55%".
    pub fn print(&self) {
        eprintln!(
            "pid: {} mem: {} CPU: {}",
            self.pid,
            self.mem_str(),
            self.cpu_str()
        );
    }
}
