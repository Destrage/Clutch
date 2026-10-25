//! Delivers canvas frames to encoders without ever stalling the video thread.
//!
//! CPU consumers get NV12 in CPU memory: conversion and copies are recorded with the
//! frame, buffers are mapped asynchronously, and frames that can't be taken (no free
//! buffer, consumer queue full) are dropped and counted. GPU consumers (hardware
//! encoders, when a [`crate::GpuExport`] is configured) get the NV12 planes on the GPU
//! instead; see [`GpuRing`].

use crate::gpu_export::{ExportPlanes, GpuExport, GpuFrame};
use crossbeam_channel::{Receiver, Sender, TrySendError};
use estream_core::{CanvasId, Timestamp};
use estream_gpu::{ConvertInput, GpuContext, Nv12Target, OutputConverter, wgpu};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

/// One NV12 frame. CPU frames hold `height` rows of Y (`width` bytes each), then
/// `height / 2` rows of interleaved CbCr (`width` bytes each); GPU frames hold no
/// bytes and carry [`VideoFrame::gpu`] instead.
pub struct VideoFrame {
    pub timestamp: Timestamp,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// Set for frames left on the GPU.
    pub gpu: Option<GpuFrame>,
    recycle: Option<Sender<Vec<u8>>>,
}

impl VideoFrame {
    /// A standalone frame from tightly packed NV12 data (not pooled), e.g. for tests.
    pub fn from_nv12(timestamp: Timestamp, width: u32, height: u32, data: Vec<u8>) -> Self {
        debug_assert_eq!(data.len(), (width * height * 3 / 2) as usize);
        Self {
            timestamp,
            width,
            height,
            data,
            gpu: None,
            recycle: None,
        }
    }

    /// A frame whose NV12 planes stay on the GPU.
    pub fn from_gpu(timestamp: Timestamp, width: u32, height: u32, gpu: GpuFrame) -> Self {
        Self {
            timestamp,
            width,
            height,
            data: Vec::new(),
            gpu: Some(gpu),
            recycle: None,
        }
    }

    pub fn y(&self) -> &[u8] {
        &self.data[..(self.width * self.height) as usize]
    }

    pub fn uv(&self) -> &[u8] {
        &self.data[(self.width * self.height) as usize..]
    }
}

impl Drop for VideoFrame {
    fn drop(&mut self) {
        // Return the buffer to the pool so steady-state delivery doesn't allocate.
        if let Some(tx) = &self.recycle {
            let _ = tx.try_send(std::mem::take(&mut self.data));
        }
    }
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VideoFrame({}x{} @ {:?})",
            self.width, self.height, self.timestamp
        )
    }
}

/// A registered encoder input for one canvas.
#[derive(Clone)]
pub struct VideoConsumer {
    pub id: u64,
    pub canvas: CanvasId,
    pub tx: Sender<VideoFrame>,
    /// Wants GPU frames. The consumer clears it to fall back to CPU frames.
    pub gpu: Arc<AtomicBool>,
}

impl VideoConsumer {
    /// Whether this consumer gets GPU frames from `export`.
    pub(crate) fn takes_gpu(&self, export: Option<&dyn GpuExport>) -> bool {
        export.is_some() && self.gpu.load(Ordering::Relaxed)
    }
}

const IDLE: u8 = 0;
const IN_FLIGHT: u8 = 1;
const MAPPED: u8 = 2;
const FAILED: u8 = 3;

struct Slot {
    target: Nv12Target,
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

/// Per-canvas ring of NV12 readback buffers.
pub(crate) struct ReadbackRing {
    pub width: u32,
    pub height: u32,
    slots: Vec<Slot>,
    /// Slots awaiting delivery, oldest first.
    pending: VecDeque<(usize, Timestamp)>,
    y_row: u32,
    uv_row: u32,
    pool_tx: Sender<Vec<u8>>,
    pool_rx: Receiver<Vec<u8>>,
}

fn align(v: u32) -> u32 {
    v.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

impl ReadbackRing {
    pub const SLOTS: usize = 4;

    /// `None` for odd sizes (NV12 needs even dimensions).
    pub fn new(gpu: &GpuContext, width: u32, height: u32) -> Option<Self> {
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            tracing::warn!(
                width,
                height,
                "canvas size is odd; encoders need even dimensions"
            );
            return None;
        }
        let (y_row, uv_row) = (align(width), align(width));
        let size = (y_row * height + uv_row * (height / 2)) as u64;
        let slots = (0..Self::SLOTS)
            .map(|_| Slot {
                target: Nv12Target::new(gpu, width, height),
                buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nv12 readback"),
                    size,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(IDLE)),
            })
            .collect();
        let (pool_tx, pool_rx) = crossbeam_channel::bounded(Self::SLOTS * 4);
        Some(Self {
            width,
            height,
            slots,
            pending: VecDeque::with_capacity(Self::SLOTS),
            y_row,
            uv_row,
            pool_tx,
            pool_rx,
        })
    }

    /// Records NV12 conversion + readback of `input` into a free slot.
    /// Returns `None` (frame dropped) if every slot is still busy.
    pub fn record(
        &mut self,
        converter: &OutputConverter,
        input: &ConvertInput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<usize> {
        let index = self
            .slots
            .iter()
            .position(|s| s.state.load(Ordering::Acquire) == IDLE)?;
        let slot = &self.slots[index];
        converter.to_nv12(encoder, input, &slot.target);
        copy_plane(
            encoder,
            &slot.target.y,
            &slot.buffer,
            0,
            self.y_row,
            self.height,
        );
        copy_plane(
            encoder,
            &slot.target.uv,
            &slot.buffer,
            (self.y_row * self.height) as u64,
            self.uv_row,
            self.height / 2,
        );
        slot.state.store(IN_FLIGHT, Ordering::Release);
        Some(index)
    }

    /// Requests the mapping after the frame's commands were submitted.
    pub fn request_map(&mut self, index: usize, timestamp: Timestamp) {
        let state = self.slots[index].state.clone();
        self.slots[index]
            .buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                state.store(
                    if result.is_ok() { MAPPED } else { FAILED },
                    Ordering::Release,
                );
            });
        self.pending.push_back((index, timestamp));
    }

    /// Hands every mapped frame (in order) to the canvas's CPU consumers.
    pub fn deliver(
        &mut self,
        consumers: &[VideoConsumer],
        canvas: CanvasId,
        export: Option<&dyn GpuExport>,
        dropped: &AtomicU64,
    ) {
        let mine = |c: &&VideoConsumer| c.canvas == canvas && !c.takes_gpu(export);
        while let Some(&(index, timestamp)) = self.pending.front() {
            let slot = &self.slots[index];
            match slot.state.load(Ordering::Acquire) {
                MAPPED => {}
                FAILED => {
                    slot.state.store(IDLE, Ordering::Release);
                    self.pending.pop_front();
                    dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                _ => return, // not ready yet; keep order
            }
            self.pending.pop_front();
            let (w, h) = (self.width as usize, self.height as usize);
            let mut data = self.pool_rx.try_recv().unwrap_or_default();
            data.clear();
            data.reserve(w * h * 3 / 2);
            if let Ok(view) = slot.buffer.get_mapped_range(..) {
                for row in 0..h {
                    let start = row * self.y_row as usize;
                    data.extend_from_slice(&view[start..start + w]);
                }
                let uv_base = self.y_row as usize * h;
                for row in 0..h / 2 {
                    let start = uv_base + row * self.uv_row as usize;
                    data.extend_from_slice(&view[start..start + w]);
                }
            }
            slot.buffer.unmap();
            slot.state.store(IDLE, Ordering::Release);

            let count = consumers.iter().filter(mine).count();
            for (i, consumer) in consumers.iter().filter(mine).enumerate() {
                let frame_data = if i + 1 == count {
                    std::mem::take(&mut data)
                } else {
                    data.clone()
                };
                let frame = VideoFrame {
                    timestamp,
                    width: self.width,
                    height: self.height,
                    data: frame_data,
                    gpu: None,
                    recycle: Some(self.pool_tx.clone()),
                };
                match consumer.tx.try_send(frame) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(TrySendError::Disconnected(_)) => {}
                }
            }
        }
    }
}

/// Per-canvas ring of NV12 planes that stay on the GPU, for consumers that take
/// [`GpuFrame`]s. A slot is reused only once every frame holding its handle was dropped;
/// before rendering into it again the queue waits (GPU side) for the consumer's last read.
pub(crate) struct GpuRing {
    pub width: u32,
    pub height: u32,
    slots: Vec<ExportPlanes>,
    /// Slots rendered this frame, waiting for the post-submit fence value.
    pending: Option<(usize, Timestamp)>,
}

impl GpuRing {
    pub const SLOTS: usize = 4;

    /// `None` for odd sizes or when the export can't create planes.
    pub fn new(gpu: &GpuContext, export: &dyn GpuExport, width: u32, height: u32) -> Option<Self> {
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return None;
        }
        let slots = (0..Self::SLOTS)
            .map(|_| export.create_planes(gpu, width, height))
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            width,
            height,
            slots,
            pending: None,
        })
    }

    /// Records NV12 conversion of `input` into a free slot. Returns `false` (frame dropped)
    /// if every slot is still held by consumers.
    pub fn record(
        &mut self,
        gpu: &GpuContext,
        export: &dyn GpuExport,
        converter: &OutputConverter,
        input: &ConvertInput,
        encoder: &mut wgpu::CommandEncoder,
        timestamp: Timestamp,
    ) -> bool {
        let Some(index) = self
            .slots
            .iter()
            .position(|s| Arc::strong_count(&s.handle) == 1)
        else {
            return false;
        };
        let slot = &self.slots[index];
        export.wait_released(gpu, &slot.handle);
        converter.to_nv12(encoder, input, &slot.target);
        self.pending = Some((index, timestamp));
        true
    }

    /// After submit: hands the frame recorded this tick to the canvas's GPU consumers.
    pub fn deliver(
        &mut self,
        consumers: &[VideoConsumer],
        canvas: CanvasId,
        export: &dyn GpuExport,
        ready: u64,
        dropped: &AtomicU64,
    ) {
        let Some((index, timestamp)) = self.pending.take() else {
            return;
        };
        let handle = &self.slots[index].handle;
        for consumer in consumers
            .iter()
            .filter(|c| c.canvas == canvas && c.takes_gpu(Some(export)))
        {
            let frame = VideoFrame::from_gpu(
                timestamp,
                self.width,
                self.height,
                GpuFrame {
                    planes: handle.clone(),
                    ready,
                },
            );
            match consumer.tx.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
}

fn copy_plane(
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    buffer: &wgpu::Buffer,
    offset: u64,
    bytes_per_row: u32,
    rows: u32,
) {
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(rows),
            },
        },
        texture.size(),
    );
}
