//! Explicit GPU → CPU readback for tests, screenshots and golden images.
//! Never used on the real-time path.

use crate::{GpuContext, GpuError};
use std::path::Path;

/// Copies a whole texture to CPU memory with tightly packed rows.
pub fn read_texture(gpu: &GpuContext, texture: &wgpu::Texture) -> Result<Vec<u8>, GpuError> {
    let format = texture.format();
    let bpp = format
        .block_copy_size(None)
        .ok_or_else(|| GpuError::Readback(format!("unsupported format {format:?}")))?;
    let (width, height) = (texture.width(), texture.height());
    let row = width * bpp;
    let padded_row =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded_row * height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    gpu.queue.submit([encoder.finish()]);

    let (tx, rx) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = tx.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| GpuError::Readback(e.to_string()))?;
    rx.recv()
        .map_err(|e| GpuError::Readback(e.to_string()))?
        .map_err(|e| GpuError::Readback(e.to_string()))?;

    let mut out = Vec::with_capacity((row * height) as usize);
    {
        let mapped = buffer
            .get_mapped_range(..)
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        for y in 0..height {
            let start = (y * padded_row) as usize;
            out.extend_from_slice(&mapped[start..start + row as usize]);
        }
    }
    buffer.unmap();
    Ok(out)
}

/// Saves tightly packed RGBA8 pixels as a PNG.
pub fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer
        .write_image_data(rgba)
        .map_err(std::io::Error::other)?;
    Ok(())
}

/// Loads an RGBA8 PNG, returning (width, height, pixels).
pub fn load_png(path: &Path) -> std::io::Result<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let mut reader = decoder.read_info().map_err(std::io::Error::other)?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let info = reader.next_frame(&mut buf).map_err(std::io::Error::other)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(std::io::Error::other("expected an RGBA8 PNG"));
    }
    buf.truncate(info.buffer_size());
    Ok((info.width, info.height, buf))
}
