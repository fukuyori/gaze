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
    #[cfg(not(windows))]
    pub fn new(ctx: &eframe::egui::Context) -> std::io::Result<Self> {
        Self::spawn(ctx, || {})
    }

    #[cfg(windows)]
    pub fn new(ctx: &eframe::egui::Context, window: isize) -> std::io::Result<Self> {
        Self::spawn(ctx, move || wake_windows_window(window))
    }

    fn spawn(
        ctx: &eframe::egui::Context,
        native_wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_ctx = ctx.clone();
        let worker = thread::Builder::new()
            .name("gaze-repaint-watchdog".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    thread::park_timeout(WAKE_INTERVAL);
                    if !worker_stop.load(Ordering::Acquire) {
                        // Keep both egui's event-loop proxy and the native
                        // window queue moving. Either path can recover if the
                        // other's scheduled repaint notification is lost.
                        worker_ctx.request_repaint();
                        native_wake();
                    }
                }
            })?;

        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

#[cfg(windows)]
fn wake_windows_window(window: isize) {
    use windows_sys::Win32::{
        Graphics::Gdi::InvalidateRect,
        UI::WindowsAndMessaging::{PostMessageW, WM_NULL},
    };

    // SAFETY: both calls only enqueue work for the HWND. If eframe has already
    // destroyed it during shutdown, Windows rejects the stale handle without
    // dereferencing application memory.
    unsafe {
        InvalidateRect(window as _, std::ptr::null(), 0);
        PostMessageW(window as _, WM_NULL, 0, 0);
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
