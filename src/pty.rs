//! A terminal session behind a trait. `RealPty` runs a child on a `portable-pty` pseudo-terminal.
//! `FakePty` records each call and returns canned output, so callers test without a process.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// A failed pty operation, carrying the underlying message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyError(pub String);

/// Spawn, drive, and stop one terminal child. `read_output` never blocks.
pub trait PtySession {
    fn spawn(&mut self, argv: &[String], rows: u16, cols: u16) -> Result<(), PtyError>;
    fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError>;
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError>;
    /// Drains whatever output has arrived since the last call; empty when there is none.
    fn read_output(&mut self) -> Vec<u8>;
    fn kill(&mut self) -> Result<(), PtyError>;
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn not_spawned() -> PtyError {
    PtyError("pty is not spawned".to_string())
}

/// The live handles of one spawned child.
struct Running {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<Vec<u8>>,
}

/// Kills a spawned child, then waits so the process is reaped and not left defunct.
/// `None` means there was nothing to stop.
fn reap(running: Option<Running>) -> Result<(), PtyError> {
    let mut running = running.ok_or_else(not_spawned)?;
    let killed = running.child.kill().map_err(|e| PtyError(e.to_string()));
    // A failed kill can leave a live child, and `wait` would then block the UI thread.
    // `try_wait` still reaps a child that has already exited.
    let _ = match killed {
        Ok(()) => running.child.wait().map(Some),
        Err(_) => running.child.try_wait(),
    };
    killed
}

/// A `PtySession` backed by the platform pty.
#[derive(Default)]
pub struct RealPty {
    running: Option<Running>,
}

impl Drop for RealPty {
    fn drop(&mut self) {
        let _ = reap(self.running.take());
    }
}

impl RealPty {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PtySession for RealPty {
    fn spawn(&mut self, argv: &[String], rows: u16, cols: u16) -> Result<(), PtyError> {
        // A second spawn must not orphan the first child.
        let _ = reap(self.running.take());
        if argv.is_empty() {
            return Err(PtyError("argv is empty".to_string()));
        }
        let pair = native_pty_system()
            .openpty(size(rows, cols))
            .map_err(|e| PtyError(e.to_string()))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError(e.to_string()))?;
        // The child spawns last. A failure after it would drop the child unkilled and unreaped.
        let command = CommandBuilder::from_argv(argv.iter().map(OsString::from).collect());
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| PtyError(e.to_string()))?;
        let (tx, output) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        self.running = Some(Running {
            master: pair.master,
            writer,
            child,
            output,
        });
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        let running = self.running.as_mut().ok_or_else(not_spawned)?;
        running
            .writer
            .write_all(bytes)
            .and_then(|()| running.writer.flush())
            .map_err(|e| PtyError(e.to_string()))
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError> {
        let running = self.running.as_ref().ok_or_else(not_spawned)?;
        running
            .master
            .resize(size(rows, cols))
            .map_err(|e| PtyError(e.to_string()))
    }

    fn read_output(&mut self) -> Vec<u8> {
        self.running
            .as_ref()
            .map(|running| running.output.try_iter().flatten().collect())
            .unwrap_or_default()
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        reap(self.running.take())
    }
}

/// One call received by a `FakePty`, with the arguments it carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyCall {
    Spawn {
        argv: Vec<String>,
        rows: u16,
        cols: u16,
    },
    Write(Vec<u8>),
    Resize {
        rows: u16,
        cols: u16,
    },
    ReadOutput,
    Kill,
}

/// A `PtySession` that spawns nothing. Every call is recorded; `read_output` returns the canned
/// bytes once and then returns empty.
#[derive(Debug, Default)]
pub struct FakePty {
    calls: Vec<PtyCall>,
    canned: Vec<u8>,
}

impl FakePty {
    pub fn with_output(canned: &[u8]) -> Self {
        Self {
            calls: Vec::new(),
            canned: canned.to_vec(),
        }
    }

    pub fn calls(&self) -> &[PtyCall] {
        &self.calls
    }
}

impl PtySession for FakePty {
    fn spawn(&mut self, argv: &[String], rows: u16, cols: u16) -> Result<(), PtyError> {
        self.calls.push(PtyCall::Spawn {
            argv: argv.to_vec(),
            rows,
            cols,
        });
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.calls.push(PtyCall::Write(bytes.to_vec()));
        Ok(())
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError> {
        self.calls.push(PtyCall::Resize { rows, cols });
        Ok(())
    }

    fn read_output(&mut self) -> Vec<u8> {
        self.calls.push(PtyCall::ReadOutput);
        std::mem::take(&mut self.canned)
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.calls.push(PtyCall::Kill);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_records_spawn_write_resize_read_and_kill_with_their_arguments() {
        let mut fake = FakePty::with_output(b"hi\r\n");
        let argv = ["sh", "-c", "echo hi"].map(String::from);
        let session: &mut dyn PtySession = &mut fake;

        session.spawn(&argv, 24, 80).unwrap();
        session.write(b"ls\n").unwrap();
        session.resize(40, 120).unwrap();
        let first = session.read_output();
        let second = session.read_output();
        session.kill().unwrap();

        assert_eq!((first, second), (b"hi\r\n".to_vec(), Vec::new()));
        assert_eq!(
            fake.calls(),
            [
                PtyCall::Spawn {
                    argv: argv.to_vec(),
                    rows: 24,
                    cols: 80
                },
                PtyCall::Write(b"ls\n".to_vec()),
                PtyCall::Resize {
                    rows: 40,
                    cols: 120
                },
                PtyCall::ReadOutput,
                PtyCall::ReadOutput,
                PtyCall::Kill,
            ]
        );
    }
}
