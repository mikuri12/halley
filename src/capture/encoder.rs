use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::thread;

use calloop::LoopHandle;
use calloop::channel::{Event, Sender as LoopSender, channel as loop_channel};

/// One image waiting to be written to disk.
pub struct EncodeJob {
    pub id: u64,
    pub directory: PathBuf,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The outcome of an [`EncodeJob`], delivered back on the compositor loop.
pub struct EncodeDone {
    pub id: u64,
    pub result: Result<PathBuf, String>,
}

/// Off-loop PNG encoder.
///
/// Compressing a multi-megapixel RGBA image takes long enough that doing it
/// inline froze every output — no input, no frames — until the file was
/// written. The GPU readback still has to happen on the compositor thread
/// because the GL context is bound to it, but everything after that is pure
/// CPU work on a plain byte buffer, so it belongs on a worker.
pub struct ScreenshotEncoder {
    jobs: Sender<EncodeJob>,
    next_id: u64,
}

impl ScreenshotEncoder {
    /// Spawns the worker and routes completions onto `loop_handle`.
    pub fn spawn<App: 'static>(
        loop_handle: &LoopHandle<'_, App>,
        on_done: impl FnMut(&mut App, EncodeDone) + 'static,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let (jobs, job_rx) = channel::<EncodeJob>();
        let (done_tx, done_rx): (LoopSender<EncodeDone>, _) = loop_channel();

        thread::Builder::new()
            .name("halley-screenshot-encoder".into())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let result = super::screenshot::write_capture(
                        &job.directory,
                        job.width,
                        job.height,
                        &job.pixels,
                    )
                    .map_err(|err| err.to_string());
                    if done_tx.send(EncodeDone { id: job.id, result }).is_err() {
                        break;
                    }
                }
            })?;

        let mut on_done = on_done;
        loop_handle.insert_source(done_rx, move |event, _, app| {
            if let Event::Msg(done) = event {
                on_done(app, done);
            }
        })?;

        Ok(Self { jobs, next_id: 0 })
    }

    /// Queues an image and returns the id its completion will carry.
    #[cfg_attr(test, allow(dead_code))]
    pub fn submit(
        &mut self,
        directory: PathBuf,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    ) -> Result<u64, String> {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        self.jobs
            .send(EncodeJob {
                id,
                directory,
                width,
                height,
                pixels,
            })
            .map_err(|_| "screenshot encoder thread is gone".to_string())?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The whole round trip: pixels handed to the worker, PNG written off the
    /// compositor thread, completion delivered back on the event loop.
    #[test]
    fn an_encoded_capture_comes_back_on_the_loop() {
        let directory = std::env::temp_dir().join(format!("halley-encoder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);

        let mut event_loop = calloop::EventLoop::<Vec<EncodeDone>>::try_new().expect("event loop");
        let mut encoder =
            ScreenshotEncoder::spawn(&event_loop.handle(), |done: &mut Vec<EncodeDone>, event| {
                done.push(event)
            })
            .expect("spawn encoder");

        let id = encoder
            .submit(directory.clone(), 2, 2, vec![255u8; 2 * 2 * 4])
            .expect("submit");

        let mut done = Vec::new();
        for _ in 0..50 {
            event_loop
                .dispatch(Some(Duration::from_millis(100)), &mut done)
                .expect("dispatch");
            if !done.is_empty() {
                break;
            }
        }

        assert_eq!(done.len(), 1, "expected exactly one completion");
        assert_eq!(done[0].id, id);
        let path = done[0].result.as_ref().expect("encode succeeded");
        assert!(path.is_file(), "{path:?} should exist");
        assert!(std::fs::metadata(path).expect("metadata").len() > 0);

        let _ = std::fs::remove_dir_all(&directory);
    }
}
