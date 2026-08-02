use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const WAKE_INTERVAL: Duration = Duration::from_millis(100);

pub struct RepaintWatchdog {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl RepaintWatchdog {
    pub fn new(ctx: &eframe::egui::Context) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_ctx = ctx.clone();
        let worker = thread::Builder::new()
            .name("gaze-repaint-watchdog".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    thread::park_timeout(WAKE_INTERVAL);
                    if !worker_stop.load(Ordering::Acquire) {
                        // request_repaint is safe from another thread and wakes
                        // winit even if its scheduled repaint deadline was lost.
                        worker_ctx.request_repaint();
                    }
                }
            })?;

        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for RepaintWatchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}
