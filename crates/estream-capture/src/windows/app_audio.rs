//! Per-application audio capture (WASAPI process loopback, Windows 10 2004+).
//!
//! Captures one application's audio (including its child processes, e.g. a
//! browser's audio service) by executable name, and re-attaches when the
//! application restarts.

use estream_audio::convert::InputConverter;
use estream_audio::ring::{StereoConsumer, stereo_ring};
use estream_audio::{AudioSource, Block, SAMPLE_RATE};
use estream_core::{SourceStatus, Timestamp};
use estream_engine::{Property, PropertySchema, SelectOption, SourceType};
use serde_json::Value;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{BLOB, COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{IUnknown, Interface, Ref, implement};

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion(SyncSender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

/// `(pid, parent pid, exe)` for every running process.
fn processes() -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    // SAFETY: standard ToolHelp iteration; the snapshot handle is closed.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                out.push((
                    entry.th32ProcessID,
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..len]),
                ));
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// The root process of an application (a matching process whose parent isn't the same app).
pub fn find_app(exe: &str) -> Option<u32> {
    let list = processes();
    let matching: Vec<&(u32, u32, String)> = list
        .iter()
        .filter(|(_, _, e)| e.eq_ignore_ascii_case(exe))
        .collect();
    matching
        .iter()
        .find(|(_, parent, _)| !matching.iter().any(|(pid, _, _)| pid == parent))
        .or(matching.first())
        .map(|(pid, _, _)| *pid)
}

fn activate(pid: u32) -> Result<IAudioClient, String> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // ManuallyDrop: PROPVARIANT's Drop would try to free our stack blob.
    let prop = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: &mut params as *mut _ as *mut u8,
                    },
                },
            }),
        },
    });
    let (tx, rx) = sync_channel(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Completion(tx).into();
    // SAFETY: `params` and `prop` outlive the activation (we wait for completion below).
    unsafe {
        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*prop as *const PROPVARIANT),
            &handler,
        )
        .map_err(|e| e.message())?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Audio activation timed out".to_owned())?;
        let mut hr = windows::core::HRESULT(0);
        let mut unknown: Option<IUnknown> = None;
        op.GetActivateResult(&mut hr, &mut unknown)
            .map_err(|e| e.message())?;
        hr.ok().map_err(|e| e.message())?;
        unknown
            .ok_or_else(|| "No audio client".to_owned())?
            .cast::<IAudioClient>()
            .map_err(|e| e.message())
    }
}

/// Captures `pid` until it exits, `stop` is set, or an error occurs.
fn capture(
    pid: u32,
    stop: &AtomicBool,
    tx: &std::sync::mpsc::Sender<StereoConsumer>,
) -> Result<(), String> {
    let client = activate(pid)?;
    let format = WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: 2,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    // SAFETY: COM calls on a valid client; handles are closed on exit.
    unsafe {
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                2_000_000, // 200 ms
                0,
                &format,
                None,
            )
            .map_err(|e| e.message())?;
        let event = CreateEventW(None, false, false, None).map_err(|e| e.message())?;
        client.SetEventHandle(event).map_err(|e| e.message())?;
        let capture: IAudioCaptureClient = client.GetService().map_err(|e| e.message())?;
        let process: HANDLE =
            OpenProcess(PROCESS_SYNCHRONIZE, false, pid).map_err(|e| e.message())?;
        let (mut producer, consumer) = stereo_ring(SAMPLE_RATE as usize / 2);
        let _ = tx.send(consumer);
        let mut converter = InputConverter::new(2, SAMPLE_RATE);
        client.Start().map_err(|e| e.message())?;
        tracing::info!(pid, "application audio capture started");

        let mut silence: Vec<f32> = Vec::new();
        let result = loop {
            if stop.load(Ordering::Acquire) {
                break Ok(());
            }
            if WaitForSingleObject(process, 0) == WAIT_OBJECT_0 {
                break Err("Application closed".to_owned());
            }
            let _ = WaitForSingleObject(event, 100);
            loop {
                let Ok(packet) = capture.GetNextPacketSize() else {
                    break;
                };
                if packet == 0 {
                    break;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                if capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }
                let samples = frames as usize * 2;
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    silence.resize(samples, 0.0);
                    converter.push(&silence[..samples], &mut producer);
                } else {
                    let slice = std::slice::from_raw_parts(data as *const f32, samples);
                    converter.push(slice, &mut producer);
                }
                let _ = capture.ReleaseBuffer(frames);
            }
        };
        let _ = client.Stop();
        let _ = CloseHandle(process);
        let _ = CloseHandle(event);
        result
    }
}

pub struct AppAudioSource {
    exe: Option<String>,
    stop: Arc<AtomicBool>,
    rx: Option<Receiver<StereoConsumer>>,
    consumer: Option<StereoConsumer>,
    status: Arc<Mutex<SourceStatus>>,
}

impl AppAudioSource {
    fn new() -> Self {
        Self {
            exe: None,
            stop: Arc::new(AtomicBool::new(true)),
            rx: None,
            consumer: None,
            status: Arc::new(Mutex::new(SourceStatus::Unavailable(
                "Choose an application".into(),
            ))),
        }
    }

    fn restart(&mut self, exe: String) {
        self.stop.store(true, Ordering::Release);
        self.consumer = None;
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let status = self.status.clone();
        let set = move |s: SourceStatus| {
            if let Ok(mut st) = status.lock() {
                *st = s;
            }
        };
        if exe.is_empty() {
            set(SourceStatus::Unavailable("Choose an application".into()));
        } else {
            let stop_flag = stop.clone();
            let name = exe.clone();
            let _ = std::thread::Builder::new()
                .name("estream-app-audio".into())
                .spawn(move || {
                    // SAFETY: per-thread COM init for WASAPI activation.
                    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                    while !stop_flag.load(Ordering::Acquire) {
                        match find_app(&name) {
                            Some(pid) => {
                                set(SourceStatus::Active);
                                if let Err(e) = capture(pid, &stop_flag, &tx) {
                                    set(SourceStatus::Unavailable(format!(
                                        "{e} — waiting for {name}"
                                    )));
                                }
                            }
                            None => set(SourceStatus::Unavailable(format!("Waiting for {name}"))),
                        }
                        for _ in 0..20 {
                            if stop_flag.load(Ordering::Acquire) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                });
        }
        self.stop = stop;
        self.rx = Some(rx);
        self.exe = Some(exe);
    }
}

impl Drop for AppAudioSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl AudioSource for AppAudioSource {
    fn update(&mut self, settings: &Value) -> Result<(), String> {
        let exe = settings
            .get("app")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if self.exe.as_deref() != Some(exe.as_str()) {
            self.restart(exe);
        }
        Ok(())
    }

    fn read(&mut self, block: &mut Block, _: Timestamp) -> usize {
        if let Some(rx) = &self.rx
            && let Ok(c) = rx.try_recv()
        {
            self.consumer = Some(c);
        }
        let Some(c) = &mut self.consumer else {
            return 0;
        };
        let available = c.available();
        if available > SAMPLE_RATE as usize * 60 / 1000 {
            c.skip(available - SAMPLE_RATE as usize * 20 / 1000);
        }
        c.read_block(block)
    }

    fn status(&self) -> SourceStatus {
        self.status
            .try_lock()
            .map(|s| s.clone())
            .unwrap_or(SourceStatus::Active)
    }
}

pub fn source_type() -> SourceType {
    SourceType::audio(
        "app_audio",
        "Application audio",
        "Capture one app's audio (games, browsers, music players)",
        |settings| {
            let mut apps: Vec<String> = super::enumerate::windows()
                .into_iter()
                .map(|w| w.exe)
                .collect();
            apps.sort_by_key(|a| a.to_lowercase());
            apps.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            let current = settings
                .get("app")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !current.is_empty() && !apps.iter().any(|a| a.eq_ignore_ascii_case(current)) {
                apps.insert(0, current.to_owned());
            }
            let options = apps
                .into_iter()
                .map(|a| SelectOption::new(a.clone(), a))
                .collect();
            PropertySchema::new(vec![
                Property::select("app", "Application", options, "")
                    .hint("Includes the app's child processes; re-attaches when it restarts"),
            ])
        },
        |settings| {
            let mut s = AppAudioSource::new();
            s.update(settings)?;
            Ok(Box::new(s))
        },
    )
    .with_validation(|_| {
        PropertySchema::new(vec![Property::select("app", "Application", vec![], "")])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_process_is_found_by_exe() {
        // The test runner itself is a running process.
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().into_owned();
        assert!(find_app(&name).is_some());
        assert!(find_app("definitely-not-running-xyz.exe").is_none());
    }

    /// Plays a short system sound in a child process and captures only that process.
    /// Opt-in because it makes a sound: `cargo test -p estream-capture app_audio -- --ignored`.
    #[test]
    #[ignore = "plays a short sound"]
    fn captures_a_single_process() {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let mut child = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Start-Sleep -Milliseconds 400; (New-Object Media.SoundPlayer 'C:/Windows/Media/Windows Ding.wav').PlaySync(); Start-Sleep -Milliseconds 300",
            ])
            .spawn()
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let pid = child.id();
        let worker = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                capture(pid, &stop, &tx)
            })
        };
        let mut consumer = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("capture started");
        let mut peak = 0.0f32;
        let mut frames = 0;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            let mut block = Block::SILENT;
            frames += consumer.read_block(&mut block);
            peak = peak.max(block.peak()[0]);
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::Release);
        let _ = child.wait();
        let _ = worker.join();
        eprintln!("captured {frames} frames, peak {peak}");
        assert!(frames > 24_000, "audio frames flowed: {frames}");
        assert!(peak > 0.01, "the ding was captured: peak {peak}");
    }
}
