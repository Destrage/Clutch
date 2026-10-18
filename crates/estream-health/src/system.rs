//! Machine load: CPU (total and Clutch), memory, GPU 3D load and free disk space, sampled
//! once a second on a background thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemSample {
    /// Whole machine, 0–100.
    pub cpu_total: f64,
    /// Clutch's share of all cores, 0–100.
    pub cpu_process: f64,
    /// Clutch's private memory.
    pub memory_bytes: u64,
    /// Busiest GPU's 3D engine load, 0–100 (`None` if unavailable).
    pub gpu: Option<f64>,
    /// Free space on the watched drive (the recording folder).
    pub disk_free: Option<u64>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Samples the machine every second on its own thread.
pub struct SystemSampler {
    latest: Arc<Mutex<SystemSample>>,
    disk_path: Arc<Mutex<Option<PathBuf>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SystemSampler {
    pub fn start() -> Self {
        let latest = Arc::new(Mutex::new(SystemSample::default()));
        let disk_path = Arc::new(Mutex::new(None::<PathBuf>));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (latest, disk_path, stop) = (latest.clone(), disk_path.clone(), stop.clone());
            std::thread::Builder::new()
                .name("estream-system-stats".into())
                .spawn(move || {
                    let mut probe = Probe::new();
                    while !stop.load(Ordering::Acquire) {
                        let path = lock(&disk_path).clone();
                        let sample = probe.sample(path.as_deref());
                        *lock(&latest) = sample;
                        for _ in 0..10 {
                            if stop.load(Ordering::Acquire) {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                })
                .ok()
        };
        Self {
            latest,
            disk_path,
            stop,
            thread,
        }
    }

    /// The most recent sample (never blocks for long).
    pub fn latest(&self) -> SystemSample {
        lock(&self.latest).clone()
    }

    /// The folder whose drive's free space is reported.
    pub fn watch_disk(&self, path: Option<PathBuf>) {
        *lock(&self.disk_path) = path;
    }
}

impl Drop for SystemSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Takes one sample; CPU and GPU are measured since the previous call.
pub struct Probe {
    #[cfg(windows)]
    imp: imp::Probe,
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            imp: imp::Probe::new(),
        }
    }

    pub fn sample(&mut self, disk: Option<&std::path::Path>) -> SystemSample {
        #[cfg(windows)]
        {
            self.imp.sample(disk)
        }
        #[cfg(not(windows))]
        {
            let _ = disk;
            SystemSample::default()
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::SystemSample;
    use std::collections::HashMap;
    use std::path::Path;
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::Win32::System::Performance::{
        PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
        PdhOpenQueryW,
    };
    use windows::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes, GetSystemTimes};
    use windows::core::{HSTRING, PCWSTR};

    fn ticks(t: FILETIME) -> u64 {
        ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
    }

    pub struct Probe {
        system: Option<(u64, u64)>,
        process: Option<(u64, std::time::Instant)>,
        cores: f64,
        gpu: Option<(PDH_HQUERY, PDH_HCOUNTER)>,
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            if let Some((query, _)) = self.gpu.take() {
                // SAFETY: closes the query we opened.
                unsafe {
                    let _ = PdhCloseQuery(query);
                }
            }
        }
    }

    impl Probe {
        pub fn new() -> Self {
            Self {
                system: None,
                process: None,
                cores: std::thread::available_parallelism().map_or(1.0, |n| n.get() as f64),
                gpu: open_gpu_query(),
            }
        }

        pub fn sample(&mut self, disk: Option<&Path>) -> SystemSample {
            SystemSample {
                cpu_total: self.cpu_total(),
                cpu_process: self.cpu_process(),
                memory_bytes: memory(),
                gpu: self.gpu_load(),
                disk_free: disk.and_then(disk_free),
            }
        }

        fn cpu_total(&mut self) -> f64 {
            let (mut idle, mut kernel, mut user) = Default::default();
            // SAFETY: plain out-parameters.
            if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }
                .is_err()
            {
                return 0.0;
            }
            // Kernel time includes idle time.
            let busy_total = (ticks(kernel) + ticks(user), ticks(idle));
            let result = match self.system {
                Some((total, idle)) if busy_total.0 > total => {
                    let dt = (busy_total.0 - total) as f64;
                    let di = busy_total.1.saturating_sub(idle) as f64;
                    ((dt - di) / dt * 100.0).clamp(0.0, 100.0)
                }
                _ => 0.0,
            };
            self.system = Some(busy_total);
            result
        }

        fn cpu_process(&mut self) -> f64 {
            let (mut created, mut exited, mut kernel, mut user) = Default::default();
            // SAFETY: the pseudo-handle of this process and plain out-parameters.
            if unsafe {
                GetProcessTimes(
                    GetCurrentProcess(),
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                )
            }
            .is_err()
            {
                return 0.0;
            }
            let used = ticks(kernel) + ticks(user);
            let now = std::time::Instant::now();
            let result = match self.process {
                Some((before, at)) => {
                    let wall = now.duration_since(at).as_secs_f64() * 1e7 * self.cores;
                    if wall > 0.0 {
                        (used.saturating_sub(before) as f64 / wall * 100.0).clamp(0.0, 100.0)
                    } else {
                        0.0
                    }
                }
                None => 0.0,
            };
            self.process = Some((used, now));
            result
        }

        fn gpu_load(&mut self) -> Option<f64> {
            let (query, counter) = self.gpu?;
            // SAFETY: our query and counter; buffers sized as PDH asks.
            unsafe {
                if PdhCollectQueryData(query) != 0 {
                    return None;
                }
                let mut size = 0u32;
                let mut count = 0u32;
                let _ = PdhGetFormattedCounterArrayW(
                    counter,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    None,
                );
                if size == 0 {
                    return Some(0.0);
                }
                let mut buffer = vec![0u8; size as usize];
                let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
                if PdhGetFormattedCounterArrayW(
                    counter,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    Some(items),
                ) != 0
                {
                    return None;
                }
                let mut per_adapter: HashMap<String, f64> = HashMap::new();
                for i in 0..count as usize {
                    let item = &*items.add(i);
                    let name = item.szName.to_string().unwrap_or_default();
                    let adapter = name
                        .split_once("luid_")
                        .and_then(|(_, rest)| rest.split_once("_phys"))
                        .map_or(String::new(), |(luid, _)| luid.to_owned());
                    *per_adapter.entry(adapter).or_default() += item.FmtValue.Anonymous.doubleValue;
                }
                Some(
                    per_adapter
                        .values()
                        .copied()
                        .fold(0.0, f64::max)
                        .clamp(0.0, 100.0),
                )
            }
        }
    }

    fn open_gpu_query() -> Option<(PDH_HQUERY, PDH_HCOUNTER)> {
        let mut query = PDH_HQUERY::default();
        let mut counter = PDH_HCOUNTER::default();
        let path = HSTRING::from(r"\GPU Engine(*engtype_3D)\Utilization Percentage");
        // SAFETY: opens a query and adds one counter; closed in Drop (or here on failure).
        unsafe {
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                return None;
            }
            if PdhAddEnglishCounterW(query, &path, 0, &mut counter) != 0 {
                let _ = PdhCloseQuery(query);
                return None;
            }
            let _ = PdhCollectQueryData(query); // rates need a first collection
        }
        Some((query, counter))
    }

    fn memory() -> u64 {
        let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
        // SAFETY: the EX struct starts with the base struct; cb tells Windows which one it is.
        let ok = unsafe {
            K32GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX)
                    .cast::<PROCESS_MEMORY_COUNTERS>(),
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            )
        };
        if ok.as_bool() {
            counters.PrivateUsage as u64
        } else {
            0
        }
    }

    fn disk_free(path: &Path) -> Option<u64> {
        // The folder may not exist yet: walk up to the nearest existing parent.
        let existing = path.ancestors().find(|p| p.exists())?;
        let mut free = 0u64;
        // SAFETY: valid path and out-parameter.
        unsafe {
            GetDiskFreeSpaceExW(
                &HSTRING::from(existing.as_os_str()),
                Some(&mut free),
                None,
                None,
            )
        }
        .ok()?;
        Some(free)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_plausible() {
        let mut probe = Probe::new();
        let temp = std::env::temp_dir();
        let _ = probe.sample(Some(&temp));
        // Burn a little CPU so the process share is measurable.
        let start = std::time::Instant::now();
        let mut x = 0u64;
        while start.elapsed() < Duration::from_millis(300) {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        }
        std::hint::black_box(x);
        let s = probe.sample(Some(&temp));
        assert!((0.0..=100.0).contains(&s.cpu_total), "{s:?}");
        assert!((0.0..=100.0).contains(&s.cpu_process), "{s:?}");
        if cfg!(windows) {
            assert!(s.memory_bytes > 0, "{s:?}");
            assert!(s.disk_free.is_some_and(|f| f > 0), "{s:?}");
            assert!(s.cpu_process > 0.0, "{s:?}");
        }
        if let Some(g) = s.gpu {
            assert!((0.0..=100.0).contains(&g));
        }
    }
}
