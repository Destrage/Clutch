//! Shared-memory channels between Clutch and its helper processes (the browser helper):
//! a seqlocked video frame and a single-writer audio ring, both in named file mappings.
//!
//! Layouts are `repr(C)` with fixed field sizes so both processes agree. Windows uses named
//! file mappings, Linux POSIX shared memory (`shm_open` + `mmap`); other platforms report an
//! error. Names are written the Windows way (`Local\clutch-…`); on Linux the namespace is
//! dropped and the region lives at `/dev/shm/clutch-…` until its creator drops it.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub enum ShmError {
    Os(String),
    TooSmall,
    Unsupported,
}

impl std::error::Error for ShmError {}

impl std::fmt::Display for ShmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShmError::Os(e) => write!(f, "shared memory: {e}"),
            ShmError::TooSmall => write!(f, "shared memory is smaller than its header says"),
            ShmError::Unsupported => write!(f, "shared memory is not supported on this platform"),
        }
    }
}

// ---------------------------------------------------------------- mapping

/// A named shared-memory region mapped into this process.
pub struct Mapping {
    ptr: *mut u8,
    len: usize,
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
    /// The POSIX name, set on the creator's side, which removes it on drop.
    #[cfg(target_os = "linux")]
    unlink: Option<std::ffi::CString>,
}

// SAFETY: the mapping is plain memory; synchronization is done with atomics in the layouts.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    /// Creates (or opens an existing) region of `len` bytes.
    #[cfg(windows)]
    pub fn create(name: &str, len: usize) -> Result<Self, ShmError> {
        use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};
        let wide = windows::core::HSTRING::from(name);
        // SAFETY: plain Win32 call with a valid name; the handle is owned by the mapping.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                (len as u64 >> 32) as u32,
                len as u32,
                &wide,
            )
        }
        .map_err(|e| ShmError::Os(e.to_string()))?;
        Self::map(handle, len)
    }

    /// Creates a region other sessions and accounts can open: `sddl` is its security
    /// descriptor, e.g. `D:(A;;GA;;;SY)(A;;GA;;;IU)`. Use a `Global\` name to share across
    /// sessions (the creator needs `SeCreateGlobalPrivilege`, which services have).
    #[cfg(windows)]
    pub fn create_with_sddl(name: &str, len: usize, sddl: &str) -> Result<Self, ShmError> {
        use windows::Win32::Foundation::{HLOCAL, INVALID_HANDLE_VALUE, LocalFree};
        use windows::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: valid SDDL string; the descriptor is freed below.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &windows::core::HSTRING::from(sddl),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|e| ShmError::Os(e.to_string()))?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let wide = windows::core::HSTRING::from(name);
        // SAFETY: plain Win32 call; the handle is owned by the mapping.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                Some(&attributes),
                PAGE_READWRITE,
                (len as u64 >> 32) as u32,
                len as u32,
                &wide,
            )
        };
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        Self::map(handle.map_err(|e| ShmError::Os(e.to_string()))?, len)
    }

    /// Opens an existing region created by another process.
    #[cfg(windows)]
    pub fn open(name: &str, len: usize) -> Result<Self, ShmError> {
        use windows::Win32::System::Memory::{FILE_MAP_ALL_ACCESS, OpenFileMappingW};
        let wide = windows::core::HSTRING::from(name);
        // SAFETY: as above.
        let handle = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, &wide) }
            .map_err(|e| ShmError::Os(e.to_string()))?;
        Self::map(handle, len)
    }

    #[cfg(windows)]
    fn map(handle: windows::Win32::Foundation::HANDLE, len: usize) -> Result<Self, ShmError> {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Memory::{FILE_MAP_ALL_ACCESS, MapViewOfFile};
        // SAFETY: maps `len` bytes of a valid mapping handle.
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, len) };
        if view.Value.is_null() {
            // SAFETY: we own the handle.
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(ShmError::Os("MapViewOfFile failed".into()));
        }
        Ok(Self {
            ptr: view.Value.cast(),
            len,
            handle,
        })
    }

    /// Creates (or opens an existing) region of `len` bytes. Dropping it removes the name, so
    /// later `open`s fail; processes that already mapped it keep their view.
    #[cfg(target_os = "linux")]
    pub fn create(name: &str, len: usize) -> Result<Self, ShmError> {
        let path = posix_name(name)?;
        // SAFETY: valid NUL-terminated name; the descriptor is closed by `map_fd`.
        let fd = unsafe { libc::shm_open(path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600) };
        if fd < 0 {
            return Err(os_error("shm_open"));
        }
        // Grow a new (empty) region; never shrink one another process already maps.
        match fd_len(fd) {
            Ok(size) if size >= len => {}
            Ok(_) => {
                // SAFETY: `fd` is a valid descriptor we opened read-write.
                if unsafe { libc::ftruncate(fd, len as libc::off_t) } != 0 {
                    let err = os_error("ftruncate");
                    // SAFETY: we own `fd`.
                    unsafe { libc::close(fd) };
                    return Err(err);
                }
            }
            Err(e) => {
                // SAFETY: we own `fd`.
                unsafe { libc::close(fd) };
                return Err(e);
            }
        }
        Self::map_fd(fd, len, Some(path))
    }

    /// Opens an existing region created by another process.
    #[cfg(target_os = "linux")]
    pub fn open(name: &str, len: usize) -> Result<Self, ShmError> {
        let path = posix_name(name)?;
        // SAFETY: valid NUL-terminated name; the descriptor is closed by `map_fd`.
        let fd = unsafe { libc::shm_open(path.as_ptr(), libc::O_RDWR, 0) };
        if fd < 0 {
            return Err(os_error("shm_open"));
        }
        Self::map_fd(fd, len, None)
    }

    /// Maps `len` bytes of `fd` and closes it (the mapping keeps the memory alive).
    #[cfg(target_os = "linux")]
    fn map_fd(
        fd: libc::c_int,
        len: usize,
        unlink: Option<std::ffi::CString>,
    ) -> Result<Self, ShmError> {
        // Touching pages past the end of the object raises SIGBUS, so check its size first.
        let mapped = match fd_len(fd) {
            Ok(size) if size < len => Err(ShmError::TooSmall),
            Ok(_) => {
                // SAFETY: maps `len` bytes of a valid, large enough descriptor.
                let ptr = unsafe {
                    libc::mmap(
                        std::ptr::null_mut(),
                        len,
                        libc::PROT_READ | libc::PROT_WRITE,
                        libc::MAP_SHARED,
                        fd,
                        0,
                    )
                };
                if ptr == libc::MAP_FAILED {
                    Err(os_error("mmap"))
                } else {
                    Ok(ptr.cast::<u8>())
                }
            }
            Err(e) => Err(e),
        };
        // SAFETY: we own `fd`; an established mapping stays valid after it is closed.
        unsafe { libc::close(fd) };
        let ptr = mapped.inspect_err(|_| {
            if let Some(path) = &unlink {
                // SAFETY: valid NUL-terminated name; we created it and nobody was told it.
                unsafe { libc::shm_unlink(path.as_ptr()) };
            }
        })?;
        Ok(Self { ptr, len, unlink })
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    pub fn create(_: &str, _: usize) -> Result<Self, ShmError> {
        Err(ShmError::Unsupported)
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    pub fn open(_: &str, _: usize) -> Result<Self, ShmError> {
        Err(ShmError::Unsupported)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn atomic_u64(&self, offset: usize) -> &AtomicU64 {
        debug_assert!(offset + 8 <= self.len && offset.is_multiple_of(8));
        // SAFETY: in bounds and 8-byte aligned (mappings are page aligned).
        unsafe { &*(self.ptr.add(offset) as *const AtomicU64) }
    }

    pub fn read_u32(&self, offset: usize) -> u32 {
        // SAFETY: in bounds; header fields are written before readers are told the name.
        unsafe { std::ptr::read_volatile(self.ptr.add(offset) as *const u32) }
    }

    pub fn write_u32(&self, offset: usize, value: u32) {
        // SAFETY: in bounds.
        unsafe { std::ptr::write_volatile(self.ptr.add(offset) as *mut u32, value) }
    }

    pub fn bytes(&self, offset: usize, len: usize) -> &[u8] {
        assert!(offset + len <= self.len);
        // SAFETY: in bounds; concurrent writes are detected by the seqlock/ring protocol.
        unsafe { std::slice::from_raw_parts(self.ptr.add(offset), len) }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn bytes_mut(&self, offset: usize, len: usize) -> &mut [u8] {
        assert!(offset + len <= self.len);
        // SAFETY: in bounds; only the single writer process calls this.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.add(offset), len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: we own the view and the handle.
        unsafe {
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
            let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.ptr.cast(),
            });
            let _ = CloseHandle(self.handle);
        }
        #[cfg(target_os = "linux")]
        // SAFETY: we own the view; the name is ours to remove when we created it.
        unsafe {
            libc::munmap(self.ptr.cast(), self.len);
            if let Some(path) = &self.unlink {
                libc::shm_unlink(path.as_ptr());
            }
        }
    }
}

/// `Local\clutch-x` (or `Global\…`, or a bare name) as a POSIX name: `/clutch-x`. Other
/// characters POSIX names can't hold become `_`.
#[cfg(target_os = "linux")]
fn posix_name(name: &str) -> Result<std::ffi::CString, ShmError> {
    let bare = name
        .strip_prefix("Local\\")
        .or_else(|| name.strip_prefix("Global\\"))
        .unwrap_or(name);
    let mut path = String::from("/");
    if !bare.starts_with("clutch-") {
        path.push_str("clutch-");
    }
    path.extend(bare.chars().map(|c| {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
            c
        } else {
            '_'
        }
    }));
    // NAME_MAX for the file under /dev/shm.
    if path.len() > 255 {
        return Err(ShmError::Os(format!("name too long: {name}")));
    }
    std::ffi::CString::new(path).map_err(|e| ShmError::Os(e.to_string()))
}

#[cfg(target_os = "linux")]
fn fd_len(fd: libc::c_int) -> Result<usize, ShmError> {
    // SAFETY: `stat` is plain data and `fd` is a valid descriptor.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return Err(os_error("fstat"));
    }
    Ok(stat.st_size as usize)
}

#[cfg(target_os = "linux")]
fn os_error(call: &str) -> ShmError {
    ShmError::Os(format!("{call}: {}", std::io::Error::last_os_error()))
}

/// Removes every region whose name starts with `prefix` (written like the names, e.g.
/// `Local\clutch-browser-1234-`): on Linux a process that crashes leaves its regions behind,
/// where Windows frees them with the process. Processes that mapped one keep their view.
pub fn remove_prefixed(prefix: &str) {
    #[cfg(target_os = "linux")]
    {
        let Ok(path) = posix_name(prefix) else { return };
        let bare = path.to_string_lossy();
        let bare = bare.trim_start_matches('/');
        let Ok(dir) = std::fs::read_dir("/dev/shm") else {
            return;
        };
        for entry in dir.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with(bare)
                && let Ok(c) = std::ffi::CString::new(format!("/{name}"))
            {
                // SAFETY: valid NUL-terminated name.
                unsafe { libc::shm_unlink(c.as_ptr()) };
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = prefix;
}

// ---------------------------------------------------------------- video frame

/// Video header: seq (u64, odd while writing), width (u32), height (u32), padded to 64 bytes.
pub const FRAME_HEADER: usize = 64;

/// Bytes needed for a `width`×`height` RGBA frame.
pub fn frame_len(width: u32, height: u32) -> usize {
    FRAME_HEADER + width as usize * height as usize * 4
}

/// Writes frames (the helper side).
pub struct FrameWriter {
    map: Mapping,
    pub width: u32,
    pub height: u32,
}

impl FrameWriter {
    pub fn create(name: &str, width: u32, height: u32) -> Result<Self, ShmError> {
        let map = Mapping::create(name, frame_len(width, height))?;
        map.write_u32(8, width);
        map.write_u32(12, height);
        map.atomic_u64(0).store(0, Ordering::Release);
        Ok(Self { map, width, height })
    }

    /// Publishes a frame: `fill` writes `width*height*4` RGBA bytes.
    pub fn write(&mut self, fill: impl FnOnce(&mut [u8])) {
        let seq = self.map.atomic_u64(0);
        let start = seq.load(Ordering::Relaxed);
        seq.store(start | 1, Ordering::Release); // odd: writing
        std::sync::atomic::fence(Ordering::Release);
        let len = self.width as usize * self.height as usize * 4;
        fill(self.map.bytes_mut(FRAME_HEADER, len));
        seq.store((start | 1) + 1, Ordering::Release); // even: done
    }
}

/// Reads frames (the Clutch side).
pub struct FrameReader {
    map: Mapping,
    last: u64,
    pub width: u32,
    pub height: u32,
}

impl FrameReader {
    pub fn open(name: &str, width: u32, height: u32) -> Result<Self, ShmError> {
        let map = Mapping::open(name, frame_len(width, height))?;
        if map.read_u32(8) != width || map.read_u32(12) != height {
            return Err(ShmError::TooSmall);
        }
        Ok(Self {
            map,
            last: 0,
            width,
            height,
        })
    }

    /// Copies a new, complete frame into `out` (resized as needed). `false` if there is no
    /// new frame or the writer was mid-frame (try again next time).
    pub fn read(&mut self, out: &mut Vec<u8>) -> bool {
        let seq = self.map.atomic_u64(0);
        let before = seq.load(Ordering::Acquire);
        if before == self.last || before & 1 == 1 || before == 0 {
            return false;
        }
        let len = self.width as usize * self.height as usize * 4;
        out.clear();
        out.extend_from_slice(self.map.bytes(FRAME_HEADER, len));
        std::sync::atomic::fence(Ordering::Acquire);
        if seq.load(Ordering::Acquire) != before {
            return false; // torn: the writer started another frame
        }
        self.last = before;
        true
    }
}

// ---------------------------------------------------------------- audio ring

/// Audio header: write position in frames (u64), capacity in frames (u32), channels (u32),
/// sample rate (u32), padded to 64 bytes. Then interleaved stereo f32.
pub const AUDIO_HEADER: usize = 64;

pub fn audio_len(capacity: u32) -> usize {
    AUDIO_HEADER + capacity as usize * 2 * 4
}

/// Writes audio (the helper side, one writer).
pub struct AudioWriter {
    map: Mapping,
    capacity: u32,
}

impl AudioWriter {
    pub fn create(name: &str, capacity: u32, rate: u32) -> Result<Self, ShmError> {
        let map = Mapping::create(name, audio_len(capacity))?;
        map.write_u32(8, capacity);
        map.write_u32(12, 2);
        map.write_u32(16, rate);
        map.atomic_u64(0).store(0, Ordering::Release);
        Ok(Self { map, capacity })
    }

    /// Appends stereo frames from planar left/right slices.
    pub fn push(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        let pos = self.map.atomic_u64(0);
        let start = pos.load(Ordering::Relaxed);
        let cap = self.capacity as u64;
        let data = self.map.bytes_mut(AUDIO_HEADER, self.capacity as usize * 8);
        for i in 0..n {
            let slot = ((start + i as u64) % cap) as usize * 8;
            data[slot..slot + 4].copy_from_slice(&left[i].to_le_bytes());
            data[slot + 4..slot + 8].copy_from_slice(&right[i].to_le_bytes());
        }
        pos.store(start + n as u64, Ordering::Release);
    }
}

/// Reads audio (the Clutch side) from its own position.
pub struct AudioReader {
    map: Mapping,
    capacity: u32,
    pos: u64,
}

impl AudioReader {
    pub fn open(name: &str) -> Result<Self, ShmError> {
        // Open the header first to learn the capacity, then map the whole ring.
        let header = Mapping::open(name, AUDIO_HEADER)?;
        let capacity = header.read_u32(8);
        drop(header);
        let map = Mapping::open(name, audio_len(capacity))?;
        // Start at the current end: only new audio is played.
        let pos = map.atomic_u64(0).load(Ordering::Acquire);
        Ok(Self { map, capacity, pos })
    }

    /// Frames ready to read (capped at the ring capacity).
    pub fn available(&self) -> usize {
        let write = self.map.atomic_u64(0).load(Ordering::Acquire);
        write.saturating_sub(self.pos).min(self.capacity as u64) as usize
    }

    /// Reads up to `left.len()` frames into planar slices. Returns the number read.
    pub fn read(&mut self, left: &mut [f32], right: &mut [f32]) -> usize {
        let write = self.map.atomic_u64(0).load(Ordering::Acquire);
        let cap = self.capacity as u64;
        if write.saturating_sub(self.pos) > cap {
            // Fell too far behind (e.g. not read for a while): skip to recent audio.
            self.pos = write - cap / 2;
        }
        let n = (write.saturating_sub(self.pos) as usize).min(left.len().min(right.len()));
        let data = self.map.bytes(AUDIO_HEADER, self.capacity as usize * 8);
        for i in 0..n {
            let slot = ((self.pos + i as u64) % cap) as usize * 8;
            let f = |o: usize| f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
            left[i] = f(slot);
            right[i] = f(slot + 4);
        }
        self.pos += n as u64;
        n
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn name(kind: &str) -> String {
        format!("Local\\estream-shm-test-{}-{kind}", std::process::id())
    }

    #[test]
    fn frames_round_trip_and_skip_repeats() {
        let n = name("frame");
        let mut writer = FrameWriter::create(&n, 4, 2).unwrap();
        let mut reader = FrameReader::open(&n, 4, 2).unwrap();
        let mut buf = Vec::new();
        assert!(!reader.read(&mut buf), "nothing written yet");
        writer.write(|px| px.fill(7));
        assert!(reader.read(&mut buf));
        assert_eq!(buf.len(), 32);
        assert!(buf.iter().all(|b| *b == 7));
        assert!(!reader.read(&mut buf), "same frame twice is not new");
        writer.write(|px| px.fill(9));
        assert!(reader.read(&mut buf));
        assert_eq!(buf[0], 9);
    }

    #[test]
    fn wrong_size_is_rejected() {
        let n = name("size");
        let _writer = FrameWriter::create(&n, 4, 2).unwrap();
        assert!(FrameReader::open(&n, 8, 8).is_err());
    }

    #[test]
    fn audio_ring_reads_in_order_and_wraps() {
        let n = name("audio");
        let mut writer = AudioWriter::create(&n, 8, 48_000).unwrap();
        let mut reader = AudioReader::open(&n).unwrap();
        let (mut l, mut r) = ([0.0f32; 16], [0.0f32; 16]);
        writer.push(&[1.0, 2.0, 3.0], &[-1.0, -2.0, -3.0]);
        assert_eq!(reader.available(), 3);
        assert_eq!(reader.read(&mut l, &mut r), 3);
        assert_eq!((l[2], r[2]), (3.0, -3.0));
        // Wrap around the 8-frame ring.
        let a: Vec<f32> = (10..16).map(|v| v as f32).collect();
        writer.push(&a, &a);
        assert_eq!(reader.read(&mut l, &mut r), 6);
        assert_eq!(&l[..6], &a[..]);
    }

    #[test]
    fn audio_reader_skips_ahead_when_far_behind() {
        let n = name("overrun");
        let mut writer = AudioWriter::create(&n, 8, 48_000).unwrap();
        let mut reader = AudioReader::open(&n).unwrap();
        let block: Vec<f32> = (0..20).map(|v| v as f32).collect();
        writer.push(&block, &block);
        let (mut l, mut r) = ([0.0f32; 32], [0.0f32; 32]);
        let got = reader.read(&mut l, &mut r);
        assert_eq!(got, 4, "only recent audio (half the ring)");
        assert_eq!(&l[..4], &[16.0, 17.0, 18.0, 19.0]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn posix_names_drop_the_namespace() {
        let p = |n: &str| posix_name(n).unwrap().into_string().unwrap();
        assert_eq!(
            p("Local\\clutch-browser-1-a-video-0"),
            "/clutch-browser-1-a-video-0"
        );
        assert_eq!(p("Global\\estream-vcam-v1"), "/clutch-estream-vcam-v1");
        assert_eq!(p("a/b c"), "/clutch-a_b_c");
        assert!(posix_name(&"x".repeat(300)).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn leftovers_of_a_crashed_writer_are_removed() {
        let n = name("leftover");
        let writer = FrameWriter::create(&n, 4, 2).unwrap();
        std::mem::forget(writer); // as if its process died
        assert!(FrameReader::open(&n, 4, 2).is_ok());
        remove_prefixed(&format!(
            "Local\\estream-shm-test-{}-left",
            std::process::id()
        ));
        assert!(FrameReader::open(&n, 4, 2).is_err(), "removed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn creator_removes_the_name_on_drop() {
        let n = name("unlink");
        let writer = FrameWriter::create(&n, 4, 2).unwrap();
        let mut reader = FrameReader::open(&n, 4, 2).unwrap();
        drop(writer);
        assert!(FrameReader::open(&n, 4, 2).is_err(), "name is gone");
        let mut buf = Vec::new();
        assert!(!reader.read(&mut buf), "an open reader keeps its view");
    }
}
