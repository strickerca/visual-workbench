//! Private persistent WGC producer. Stdout is only the bounded pipe protocol.
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[cfg(windows)]
use vw_capture::stream::{ProducerReply, StreamProducer, StreamSource};
use vw_capture::{
    Error, Result,
    stream::PROCESS_MEMORY_BYTES,
    stream_wire::{self, Command, Header},
};
use vw_stream::{Cancellation, SessionBudget};

fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let started = Instant::now();
    // No active operation exists while a healthy helper waits for a command.
    let deadline = Arc::new(AtomicU64::new(u64::MAX));
    let watchdog = deadline.clone();
    std::thread::Builder::new()
        .name("vw-stream-watchdog".into())
        .spawn(move || {
            loop {
                if operation_expired(
                    started.elapsed().as_millis(),
                    watchdog.load(Ordering::Acquire),
                ) {
                    std::process::exit(2);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
        .map_err(|_| Error::Platform)?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    // First header is bounded before an untrusted request can choose a budget.
    // The bootstrap charge retires before the endpoint's own pool is created.
    let bootstrap =
        SessionBudget::new(PROCESS_MEMORY_BYTES).map_err(vw_capture::stream::stream_error)?;
    let wire_cancel = Cancellation::default();
    let first = stream_wire::read_command(&mut input, &bootstrap, &wire_cancel)?;
    let Command::Open { request } = &first.command else {
        return Err(Error::Invalid);
    };
    deadline.store(
        now_ms_any(started)?
            .checked_add(10_000)
            .ok_or(Error::Limit)?,
        Ordering::Release,
    );
    if let Err(error) = request.validate() {
        stream_wire::write_packet(
            &mut output,
            &Header::Refused {
                sequence: 0,
                connection_epoch: request.connection_epoch,
                source_generation: request.source_generation,
                error,
            },
            &[],
            &bootstrap,
            &wire_cancel,
        )?;
        return Ok(());
    }
    let request = request.clone();
    drop(first);
    drop(bootstrap);
    let budget =
        SessionBudget::new(request.endpoint_bytes).map_err(vw_capture::stream::stream_error)?;
    let open = Command::Open {
        request: request.clone(),
    };
    let request_hash = stream_wire::command_hash(&open, &budget, &wire_cancel)?;
    #[cfg(not(windows))]
    {
        stream_wire::write_packet(
            &mut output,
            &Header::Refused {
                sequence: 0,
                connection_epoch: request.connection_epoch,
                source_generation: request.source_generation,
                error: Error::Unsupported,
            },
            &[],
            &budget,
            &wire_cancel,
        )?;
        let _ = (deadline, request_hash);
        Ok(())
    }
    #[cfg(windows)]
    {
        use vw_capture::windows::stream::WgcStream;
        struct Runtime;
        impl Drop for Runtime {
            fn drop(&mut self) {
                // SAFETY: this thread successfully entered its own MTA.
                unsafe {
                    ::windows::Win32::System::WinRT::RoUninitialize();
                }
            }
        }
        // SAFETY: only this private helper thread's COM apartment changes.
        unsafe {
            ::windows::Win32::System::WinRT::RoInitialize(
                ::windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            )
            .map_err(|_| Error::Platform)?;
        }
        let _runtime = Runtime;
        let _job = vw_capture::stream_process::confine_stream_helper()?;
        let mut producer = match StreamProducer::open(request.clone(), budget.clone()) {
            Ok(value) => value,
            Err(error) => {
                stream_wire::write_packet(
                    &mut output,
                    &Header::Refused {
                        sequence: 0,
                        connection_epoch: request.connection_epoch,
                        source_generation: request.source_generation,
                        error,
                    },
                    &[],
                    &budget,
                    &wire_cancel,
                )?;
                return Ok(());
            }
        };
        let mut source = match WgcStream::open(request.clone(), budget.clone(), &wire_cancel) {
            Ok(value) => value,
            Err(error) => {
                stream_wire::write_packet(
                    &mut output,
                    &Header::Refused {
                        sequence: 0,
                        connection_epoch: request.connection_epoch,
                        source_generation: request.source_generation,
                        error,
                    },
                    &[],
                    &budget,
                    &wire_cancel,
                )?;
                return Ok(());
            }
        };
        stream_wire::write_packet(
            &mut output,
            &Header::Ready {
                request_hash,
                connection_epoch: request.connection_epoch,
                source_generation: request.source_generation,
                ring_frames: vw_capture::stream::STREAM_RING_FRAMES,
                ring_bytes: request.ring_bytes,
                endpoint_bytes: request.endpoint_bytes,
                border_visible: true,
            },
            &[],
            &budget,
            &wire_cancel,
        )?;
        let mut sequence = 0u64;
        loop {
            deadline.store(u64::MAX, Ordering::Release);
            let received = stream_wire::read_command(&mut input, &budget, &wire_cancel)?;
            deadline.store(
                now_ms(started)?.checked_add(10_000).ok_or(Error::Limit)?,
                Ordering::Release,
            );
            let command = &received.command;
            let next = command
                .sequence()
                .filter(|n| *n > sequence)
                .ok_or(Error::Invalid)?;
            if command.binding() != Some((request.connection_epoch, request.source_generation)) {
                return Err(Error::Stale);
            }
            sequence = next;
            let result = (|| -> Result<bool> {
                match command {
                    Command::Open { .. } => Err(Error::Invalid),
                    Command::Stop { .. } => {
                        producer.cancel();
                        source.discard();
                        stream_wire::write_packet(
                            &mut output,
                            &Header::Stopped {
                                sequence,
                                connection_epoch: request.connection_epoch,
                                source_generation: request.source_generation,
                            },
                            &[],
                            &budget,
                            &wire_cancel,
                        )?;
                        Ok(false)
                    }
                    Command::Original { identity, .. } => {
                        let response = producer.original(identity)?;
                        if let ProducerReply::Original { identity, encoded } = response {
                            stream_wire::write_packet(
                                &mut output,
                                &Header::Original {
                                    sequence,
                                    connection_epoch: request.connection_epoch,
                                    source_generation: request.source_generation,
                                    identity,
                                    asset_id: encoded.asset_id().to_string(),
                                    payload_bytes: u32::try_from(encoded.bytes().len())
                                        .map_err(|_| Error::Limit)?,
                                },
                                encoded.bytes(),
                                &budget,
                                &wire_cancel,
                            )?;
                            Ok(true)
                        } else {
                            Err(Error::Invalid)
                        }
                    }
                    Command::InspectSource { .. } => {
                        let ProducerReply::Idle {
                            retained_frames,
                            retained_bytes,
                        } = producer.inspect(&mut source)?
                        else {
                            return Err(Error::Invalid);
                        };
                        stream_wire::write_packet(
                            &mut output,
                            &Header::Idle {
                                sequence,
                                connection_epoch: request.connection_epoch,
                                source_generation: request.source_generation,
                                retained_frames,
                                retained_bytes,
                            },
                            &[],
                            &budget,
                            &wire_cancel,
                        )?;
                        Ok(true)
                    }
                    Command::Poll {
                        feedback,
                        repair_keyframe,
                        ..
                    } => {
                        let response = producer.poll(
                            &mut source,
                            sequence,
                            *feedback,
                            *repair_keyframe,
                            now_ms(started)?,
                        )?;
                        match response {
                            ProducerReply::Idle {
                                retained_frames,
                                retained_bytes,
                            } => {
                                stream_wire::write_packet(
                                    &mut output,
                                    &Header::Idle {
                                        sequence,
                                        connection_epoch: request.connection_epoch,
                                        source_generation: request.source_generation,
                                        retained_frames,
                                        retained_bytes,
                                    },
                                    &[],
                                    &budget,
                                    &wire_cancel,
                                )?;
                            }
                            ProducerReply::Tiles {
                                frame_id,
                                timestamp_ns,
                                captured_at_ms,
                                keyframe,
                                payload,
                            } => {
                                stream_wire::write_packet(
                                    &mut output,
                                    &Header::Tiles {
                                        sequence,
                                        connection_epoch: request.connection_epoch,
                                        source_generation: request.source_generation,
                                        frame_id,
                                        timestamp_ns,
                                        captured_at_ms,
                                        keyframe,
                                        payload_bytes: u32::try_from(payload.bytes().len())
                                            .map_err(|_| Error::Limit)?,
                                    },
                                    payload.bytes(),
                                    &budget,
                                    &wire_cancel,
                                )?;
                                producer.published(sequence)?;
                            }
                            ProducerReply::Original { .. } => return Err(Error::Invalid),
                        }
                        Ok(true)
                    }
                }
            })();
            match result {
                Ok(true) => (),
                Ok(false) => return Ok(()),
                Err(error) => {
                    stream_wire::write_packet(
                        &mut output,
                        &Header::Refused {
                            sequence,
                            connection_epoch: request.connection_epoch,
                            source_generation: request.source_generation,
                            error,
                        },
                        &[],
                        &budget,
                        &wire_cancel,
                    )?;
                    if error != Error::Unavailable {
                        return Ok(());
                    }
                }
            }
        }
    }
}
fn operation_expired(elapsed_ms: u128, deadline_ms: u64) -> bool {
    deadline_ms != u64::MAX && elapsed_ms > u128::from(deadline_ms)
}
fn now_ms_any(started: Instant) -> Result<u64> {
    u64::try_from(started.elapsed().as_millis()).map_err(|_| Error::Limit)
}
#[cfg(windows)]
fn now_ms(started: Instant) -> Result<u64> {
    now_ms_any(started)
}
#[cfg(test)]
mod tests {
    use super::operation_expired;
    #[test]
    fn idle_wait_has_no_watchdog_deadline_but_active_work_does() {
        assert!(!operation_expired(86_400_000, u64::MAX));
        assert!(!operation_expired(5_000, 5_000));
        assert!(operation_expired(5_001, 5_000));
    }
}
