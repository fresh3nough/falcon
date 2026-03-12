//! PTY Manager — spawns and manages pseudo-terminal child processes.
//!
//! Uses `portable-pty` to create a cross-platform PTY pair, then runs a
//! reader loop that forwards output chunks to the Tauri frontend via events.
//! Each PTY instance has a unique `pty_id` used in event names so multiple
//! terminals can operate independently:
//!   - `pty-output-{pty_id}` — streamed terminal output
//!   - `pty-exit-{pty_id}`   — terminal process exited

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

/// Holds the writable master-end of the PTY so Tauri commands can send
/// keystrokes from the frontend into the shell.
pub struct PtyManager {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Keep master alive so the PTY does not close.
    _master: Box<dyn MasterPty + Send>,
}

impl PtyManager {
    /// Spawn a new shell inside a PTY of the given dimensions.
    ///
    /// * `app`    — Tauri app handle used to emit `pty-output-{pty_id}` events.
    /// * `rows` / `cols` — initial terminal size.
    /// * `shell`  — path to the shell binary (e.g. `/bin/bash`).
    /// * `pty_id` — unique identifier for this PTY instance (e.g. `"main"`).
    pub fn spawn(
        app: AppHandle,
        rows: u16,
        cols: u16,
        shell: &str,
        pty_id: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let pty_system = native_pty_system();

        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");

        // Spawn the shell child process inside the PTY.
        let _child = pair.slave.spawn_command(cmd)?;
        // Drop slave so reads on the master will see EOF when the child exits.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));

        // Background task: read PTY output and forward it to the WebView.
        Self::start_reader(app, reader, pty_id);

        Ok(Self {
            writer,
            _master: pair.master,
        })
    }

    /// Write raw bytes (keystrokes) into the PTY.
    pub fn write(&self, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        let mut w = self.writer.lock().map_err(|e| e.to_string())?;
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    /// Resize the PTY grid.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), Box<dyn std::error::Error>> {
        // Resize is handled through the master PtySize — portable-pty
        // re-applies the size on the master fd.
        self._master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Spawn a blocking reader thread that emits `pty-output-{pty_id}` events.
    fn start_reader(app: AppHandle, mut reader: Box<dyn Read + Send>, pty_id: String) {
        let output_event = format!("pty-output-{pty_id}");
        let exit_event = format!("pty-exit-{pty_id}");
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF — shell exited.
                        let _ = app.emit(&exit_event, ());
                        break;
                    }
                    Ok(n) => {
                        // Send raw bytes as a UTF-8-lossy string to the frontend.
                        let text = String::from_utf8_lossy(&buf[..n]).to_string();
                        let _ = app.emit(&output_event, text);
                    }
                    Err(_) => break,
                }
            }
        });
    }
}
