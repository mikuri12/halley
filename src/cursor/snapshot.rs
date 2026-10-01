use std::{ptr, rc::Rc};

use smithay::backend::allocator::{Fourcc, format::get_bpp};
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_surface::WlSurface};
use smithay::utils::{Buffer, Rectangle, Transform};
use smithay::wayland::compositor::{BufferAssignment, SurfaceAttributes, with_states};
use smithay::wayland::shm::{shm_format_to_fourcc, with_buffer_contents};

use super::{CursorManager, CursorSurfaceSnapshot};

/// Snapshot client-provided SHM cursors before Smithay consumes the pending
/// buffer. Smithay's renderer damage history is surface-scoped, so a
/// replacement buffer can inherit damage larger than its new texture and
/// `glTexSubImage2D` a 24×24 cursor into a leftover 10×16 GLES cache.
pub fn prepare_commit(manager: &CursorManager, committed: &WlSurface) {
    if manager.client_surface() != Some(committed) {
        return;
    }

    let pending = with_states(committed, |states| {
        let mut attributes = states.cached_state.get::<SurfaceAttributes>();
        let attributes = attributes.current();
        match attributes.buffer.as_ref() {
            Some(BufferAssignment::NewBuffer(buffer)) => Some(Some((
                buffer.clone(),
                attributes.buffer_scale.max(1),
                Transform::from(attributes.buffer_transform),
            ))),
            Some(BufferAssignment::Removed) => Some(None),
            None => None,
        }
    });

    let Some(pending) = pending else {
        return;
    };
    let Some((buffer, scale, transform)) = pending else {
        manager.surface_snapshot().borrow_mut().take();
        return;
    };

    let snapshot = match copy_shm_cursor(&buffer) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            manager.surface_snapshot().borrow_mut().take();
            return;
        }
        Err(err) => {
            eventline::warn!("cursor: rejected invalid SHM snapshot: {err}");
            manager.surface_snapshot().borrow_mut().take();
            return;
        }
    };

    let mut slot = manager.surface_snapshot().borrow_mut();
    let old_size = slot.as_ref().map(|old| (old.width, old.height, old.scale));
    let reusable = slot.as_ref().filter(|old| {
        old.surface == *committed
            && old.format == snapshot.format
            && old.scale == scale
            && old.transform == transform
            && snapshot_storage_size_matches(
                (old.width, old.height),
                (snapshot.width, snapshot.height),
            )
    });
    let mut render_buffer = reusable.map(|old| old.buffer.clone()).unwrap_or_else(|| {
        MemoryRenderBuffer::new(
            snapshot.format,
            (snapshot.width as i32, snapshot.height as i32),
            scale,
            transform,
            None,
        )
    });
    {
        let mut render = render_buffer.render();
        render.resize((snapshot.width as i32, snapshot.height as i32));
        let result: Result<(), std::convert::Infallible> = render.draw(|target| {
            target.copy_from_slice(&snapshot.pixels);
            Ok(vec![Rectangle::<i32, Buffer>::from_size(
                (snapshot.width as i32, snapshot.height as i32).into(),
            )])
        });
        let _ = result;
    }

    if old_size != Some((snapshot.width, snapshot.height, scale)) {
        eventline::debug!(
            "cursor: SHM snapshot resized old={old_size:?} new={}x{} scale={} format={:?}",
            snapshot.width,
            snapshot.height,
            scale,
            snapshot.format
        );
    }
    *slot = Some(Rc::new(CursorSurfaceSnapshot {
        surface: committed.clone(),
        buffer: render_buffer,
        width: snapshot.width,
        height: snapshot.height,
        scale,
        format: snapshot.format,
        transform,
    }));
}

fn snapshot_storage_size_matches(old: (u32, u32), new: (u32, u32)) -> bool {
    old == new
}

struct ShmCursorCopy {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    format: Fourcc,
}

fn copy_shm_cursor(buffer: &WlBuffer) -> Result<Option<ShmCursorCopy>, String> {
    with_buffer_contents(buffer, |source, pool_len, data| {
        let Some(format) = shm_format_to_fourcc(data.format) else {
            return Ok(None);
        };
        let Some(bits_per_pixel) = get_bpp(format) else {
            return Ok(None);
        };
        if data.width <= 0 || data.height <= 0 || data.offset < 0 || data.stride <= 0 {
            return Err(format!("invalid geometry {data:?}"));
        }
        let bytes_per_pixel = bits_per_pixel / 8;
        let width = usize::try_from(data.width).map_err(|_| "width overflow")?;
        let height = usize::try_from(data.height).map_err(|_| "height overflow")?;
        let stride = usize::try_from(data.stride).map_err(|_| "stride overflow")?;
        let offset = usize::try_from(data.offset).map_err(|_| "offset overflow")?;
        let row_bytes = width
            .checked_mul(bytes_per_pixel)
            .ok_or("row size overflow")?;
        if stride < row_bytes {
            return Err(format!("stride {stride} smaller than row {row_bytes}"));
        }
        let required = offset
            .checked_add(
                stride
                    .checked_mul(height - 1)
                    .ok_or("buffer size overflow")?,
            )
            .and_then(|value| value.checked_add(row_bytes))
            .ok_or("buffer size overflow")?;
        if required > pool_len {
            return Err(format!("buffer end {required} exceeds pool {pool_len}"));
        }

        let mut pixels = vec![0_u8; row_bytes * height];
        for row in 0..height {
            // SAFETY: Both ranges were checked above. The SHM accessor keeps
            // the source pointer valid for the duration of this callback.
            unsafe {
                ptr::copy_nonoverlapping(
                    source.add(offset + row * stride),
                    pixels.as_mut_ptr().add(row * row_bytes),
                    row_bytes,
                );
            }
        }
        Ok(Some(ShmCursorCopy {
            pixels,
            width: width as u32,
            height: height as u32,
            format,
        }))
    })
    .map_err(|err| err.to_string())?
}

#[cfg(test)]
mod tests {
    use super::snapshot_storage_size_matches;

    #[test]
    fn resized_cursor_gets_new_renderer_storage() {
        assert!(snapshot_storage_size_matches((24, 24), (24, 24)));
        assert!(!snapshot_storage_size_matches((10, 16), (24, 24)));
        assert!(!snapshot_storage_size_matches((24, 24), (10, 16)));
    }
}
