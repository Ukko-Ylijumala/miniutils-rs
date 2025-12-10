// Copyright (c) 2024-2025 Mikko Tanner. All rights reserved.

use crate::HumanBytes as HuB;
use parking_lot::RwLock;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const MIN_INTERVAL_MS: u64 = 200;
const MAX_INTERVAL_MS: u64 = 5000;

/// ProcessInfoInner contains the actual (mutable) process information.
struct ProcessInfoInner {
    sys: System,
    mem: u64,
    cpu: f32,
    upd: Instant,
    ival: Duration,
}

/// Information about the current process.
pub struct ProcessInfo {
    pub pid: usize,
    p: Pid,
    inner: RwLock<ProcessInfoInner>,
    kind: ProcessRefreshKind,
}

impl ProcessInfo {
    pub fn new() -> Self {
        // Get the current process ID
        let pid: usize = std::process::id() as usize;
        let s_p: Pid = Pid::from(pid);
        let kind: ProcessRefreshKind = ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu()
            .without_tasks();

        // Create a System object to query system information
        let mut sys: System = System::new();
        // Do the initial refresh of the process info already here.
        refresh_processes(&mut sys, &[s_p], &kind);
        Self {
            pid,
            p: s_p,
            inner: RwLock::new(ProcessInfoInner {
                mem: sys.process(s_p).map_or_else(|| 0, |p| p.memory()),
                cpu: sys.process(s_p).map_or_else(|| 0.0, |p| p.cpu_usage()),
                sys,
                upd: Instant::now(),
                ival: Duration::from_millis(MIN_INTERVAL_MS),
            }),
            kind,
        }
    }

    /// Build with minimum interval between process info updates.
    pub fn with_min_interval(self, ival: u64) -> Self {
        self.set_interval(ival);
        self
    }

    /// Refresh the inner process info struct (at most, once every 200 ms)
    fn refresh(&self) {
        {
            let i = self.inner.read();
            if i.upd.elapsed() < i.ival {
                return;
            }
        }
        let mut i = self.inner.write();
        refresh_processes(&mut i.sys, &[self.p], &self.kind);
        i.mem = i.sys.process(self.p).map_or_else(|| 0, |p| p.memory());
        i.cpu = i.sys.process(self.p).map_or_else(|| 0.0, |p| p.cpu_usage());
        i.upd = Instant::now();
    }

    /// Set minimum interval between process info updates in milliseconds. Accepts
    /// values between 200 and 5000 ms. Lower bound is enforced since polling at
    /// higher frequencies is counterproductive and could also produce inaccurate
    /// values due to how [sysinfo::System] does its own polling.
    pub fn set_interval(&self, min_interval: u64) {
        let min_interval: u64 = min_interval.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS);
        self.inner.write().ival = Duration::from_millis(min_interval);
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

/// Refresh the [sysinfo::System] object for given processes only.
fn refresh_processes(sys: &mut System, pids: &[Pid], kind: &ProcessRefreshKind) {
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(pids), true, *kind);
}
