//! Animated braille spinner for progress indication.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crossterm::style::{Color, ResetColor, SetForegroundColor};

const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// An animated spinner that runs on a background thread.
pub struct Spinner {
    running: Arc<AtomicBool>,
    message: String,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Spinner {
    pub fn new(message: &str) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            message: message.to_string(),
            handle: Mutex::new(None),
        }
    }

    /// Start spinning on stderr.
    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let message = self.message.clone();

        let join = std::thread::spawn(move || {
            let mut idx = 0;
            let mut stderr = std::io::stderr();
            while running.load(Ordering::SeqCst) {
                let frame = FRAMES[idx % FRAMES.len()];
                let _ = write!(
                    stderr,
                    "\r{fg}{frame} {message}{reset}",
                    fg = SetForegroundColor(Color::Cyan),
                    reset = ResetColor,
                );
                let _ = stderr.flush();
                idx += 1;
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
        });

        *self.handle.lock().unwrap() = Some(join);
    }

    /// Stop with a success mark.
    pub fn stop_success(&self) {
        self.stop_with_icon("✔", Color::Green);
    }

    /// Stop with an error mark.
    pub fn stop_error(&self) {
        self.stop_with_icon("✘", Color::Red);
    }

    fn stop_with_icon(&self, icon: &str, color: Color) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.lock().unwrap().take() {
            let _ = h.join();
        }
        eprint!(
            "\r{fg}{icon} Done{reset}                    \n",
            fg = SetForegroundColor(color),
            reset = ResetColor,
        );
    }
}
