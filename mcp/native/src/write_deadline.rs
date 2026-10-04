//! One watchdog per owned bridge, independent of its potentially blocked stdout.
use crate::Result;
use std::{
    io::Write,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    deadline: Option<Instant>,
    stopped: bool,
}
pub struct WriteDeadline {
    shared: Arc<(Mutex<State>, Condvar)>,
    worker: Option<JoinHandle<()>>,
    timeout: Duration,
}
impl WriteDeadline {
    pub fn new(timeout: Duration, expired: impl FnOnce() + Send + 'static) -> Result<Self> {
        if timeout.is_zero() {
            return Err("write_timeout");
        }
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let watch = shared.clone();
        let worker = thread::Builder::new()
            .name("vw-mcp-output-deadline".into())
            .spawn(move || {
                let (lock, changed) = &*watch;
                let mut state = match lock.lock() {
                    Ok(value) => value,
                    Err(_) => {
                        expired();
                        return;
                    }
                };
                loop {
                    if state.stopped {
                        return;
                    }
                    if let Some(deadline) = state.deadline {
                        let Some(remaining) = deadline.checked_duration_since(Instant::now())
                        else {
                            drop(state);
                            expired();
                            return;
                        };
                        state = match changed.wait_timeout(state, remaining) {
                            Ok((value, _)) => value,
                            Err(_) => {
                                expired();
                                return;
                            }
                        };
                    } else {
                        state = match changed.wait(state) {
                            Ok(value) => value,
                            Err(_) => {
                                expired();
                                return;
                            }
                        };
                    }
                }
            })
            .map_err(|_| "watchdog")?;
        Ok(Self {
            shared,
            worker: Some(worker),
            timeout,
        })
    }
    pub fn frame(&self, output: &mut impl Write, bytes: &[u8]) -> Result<()> {
        if bytes.len() > crate::WIRE_BYTES {
            return Err("frame_limit");
        }
        let (lock, changed) = &*self.shared;
        {
            let mut state = lock.lock().map_err(|_| "watchdog")?;
            if state.stopped || state.deadline.is_some() {
                return Err("watchdog");
            }
            state.deadline = Some(
                Instant::now()
                    .checked_add(self.timeout)
                    .ok_or("write_timeout")?,
            );
            changed.notify_one();
        }
        let result = output
            .write_all(bytes)
            .and_then(|()| output.write_all(b"\n"))
            .and_then(|()| output.flush())
            .map_err(|_| "stdout");
        lock.lock().map_err(|_| "watchdog")?.deadline = None;
        changed.notify_one();
        result
    }
}
impl Drop for WriteDeadline {
    fn drop(&mut self) {
        let (lock, changed) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.stopped = true;
            changed.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, sync::mpsc};
    struct Blocked {
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    }
    impl Write for Blocked {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let _ = self.entered.try_send(());
            self.release
                .recv()
                .map_err(|_| io::Error::other("fixture"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn blocked_output_fires_independently_and_retains_the_exact_writer()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (expired, observed) = mpsc::sync_channel(1);
        let (entered, writing) = mpsc::sync_channel(1);
        let (release, resume) = mpsc::sync_channel(2);
        let writer = thread::spawn(move || -> Result<()> {
            let guard = WriteDeadline::new(Duration::from_millis(20), move || {
                let _ = expired.send(());
            })?;
            guard.frame(
                &mut Blocked {
                    entered,
                    release: resume,
                },
                b"bounded",
            )
        });
        writing.recv_timeout(Duration::from_secs(5))?;
        let fired = observed.recv_timeout(Duration::from_secs(5));
        // Release both writes even after a failed assertion, so the fixture never
        // leaks its exact worker. Production expiry terminates only the bridge.
        release.send(())?;
        release.send(())?;
        let result = writer.join().map_err(|_| "fixture worker")?;
        fired?;
        result?;
        Ok(())
    }
    #[test]
    fn completed_output_disarms_the_watchdog() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        let (expired, observed) = mpsc::sync_channel(1);
        let mut output = Vec::new();
        {
            let guard = WriteDeadline::new(Duration::from_secs(5), move || {
                let _ = expired.send(());
            })?;
            guard.frame(&mut output, b"{}").map_err(|_| "frame")?;
        }
        assert_eq!(output, b"{}\n");
        assert!(observed.try_recv().is_err());
        Ok(())
    }
}
