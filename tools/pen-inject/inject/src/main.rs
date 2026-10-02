#[cfg(windows)]
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    time::{Duration, Instant},
};
#[cfg(windows)]
use vw_pen_probe::platform;
#[cfg(windows)]
use vw_pen_probe::{Rect, script};

#[cfg(windows)]
const HEADER: &str = "sequence,stroke,due_ms,actual_ms,phase,x,y,pressure,tilt_x,tilt_y,rotation,pen_flags,pointer_flags\n";
#[cfg(windows)]
fn write_sample(
    file: &mut BufWriter<std::fs::File>,
    s: script::Sample,
    elapsed: f64,
    flags: u32,
) -> Result<(), String> {
    writeln!(
        file,
        "{},{},{},{:.3},{},{},{},{},{},{},{},{},{}",
        s.sequence,
        s.stroke,
        s.due_ms,
        elapsed,
        s.phase.name(),
        s.x,
        s.y,
        s.pressure,
        s.tilt_x,
        s.tilt_y,
        s.rotation,
        s.pen_flags,
        flags
    )
    .map_err(|_| "Cannot persist command journal".into())
}
#[cfg(windows)]
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--guard-trials") {
        return guard_trials(&args);
    }
    if args.len() == 2 && args[0] == "--plan" {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[1])
            .map_err(|_| "Choose a new plan output path")?;
        let mut file = BufWriter::with_capacity(256 * 1024, file);
        file.write_all(HEADER.as_bytes())
            .map_err(|_| "Cannot write plan")?;
        for s in script::generate(
            Rect {
                left: 0,
                top: 0,
                right: 800,
                bottom: 600,
            },
            false,
        )? {
            write_sample(&mut file, s, 0.0, platform::pointer_flags(s.phase))?;
        }
        file.flush().map_err(|_| "Cannot flush plan")?;
        println!("Plan written; no window queried and no input injected.");
        return Ok(());
    }
    if args.len() != 6
        || args[0] != "--run"
        || args[3] != "--owner-ready"
        || !matches!(args[5].as_str(), "normal" | "no-refresh")
    {
        return Err("Usage: pen-inject --plan <new.csv> | --run <decimal-hwnd> <pid> --owner-ready <new.csv> <normal|no-refresh>".into());
    }
    let target_hwnd = args[1].parse::<usize>().map_err(|_| "Invalid HWND")?;
    let pid = args[2].parse::<u32>().map_err(|_| "Invalid PID")?;
    platform::initialize_dpi()?;
    let baseline = platform::snapshot(target_hwnd, pid)?;
    let samples = script::generate(baseline.client, args[5] == "no-refresh")?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[4])
        .map_err(|_| "Choose a new command journal path")?;
    let mut file = BufWriter::with_capacity(256 * 1024, file);
    let session_path = std::path::Path::new(&args[4]).with_extension("session.json");
    if session_path == std::path::Path::new(&args[4]) {
        return Err("Command journal must not use .session.json extension".into());
    }
    // Empty until completion: interrupted runs cannot become acceptance evidence.
    let mut receipt = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(session_path)
        .map_err(|_| "Choose a new session receipt path")?;
    file.write_all(HEADER.as_bytes())
        .map_err(|_| "Cannot write journal")?;
    // The owner-ready HIL run may activate its own harness. Arbitrary editor targets
    // still require manual focus; there is no Windows focus-lock bypass.
    if platform::require_harness(target_hwnd, pid).is_ok() {
        platform::request_foreground_handoff(baseline)?;
        platform::focus_harness(baseline)?;
    }
    for remaining in (1..=5).rev() {
        println!(
            "Starting in {remaining}s: focus the chosen target, then leave mouse/keyboard alone. Escape in harness closes it."
        );
        std::thread::sleep(Duration::from_secs(1));
    }
    if platform::snapshot(target_hwnd, pid)? != baseline {
        return Err("Target changed during countdown; run again explicitly".into());
    }
    let mut injector = platform::Injector::arm(baseline, 1, args[5] == "no-refresh")?;
    let _priority = platform::input_priority()?;
    let start = Instant::now();
    let mut last_contact: Option<Instant> = None;
    let sample_count = samples.len();
    for s in samples {
        let deadline = start + Duration::from_millis(s.due_ms);
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        // A slow machine must not silently claim the requested keepalive rate.
        if args[5] == "normal"
            && last_contact.is_some_and(|last| last.elapsed() > Duration::from_millis(50))
        {
            return Err(format!(
                "Input paused: scheduler missed the 50 ms contact deadline (gap {:.3} ms, schedule lateness {:.3} ms)",
                last_contact.map_or(0.0, |last| last.elapsed().as_secs_f64() * 1000.0),
                Instant::now()
                    .saturating_duration_since(deadline)
                    .as_secs_f64()
                    * 1000.0
            ));
        }
        let submitted = Instant::now();
        injector.send(s)?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        write_sample(&mut file, s, elapsed, platform::pointer_flags(s.phase))?;
        last_contact = if matches!(s.phase, script::Phase::Down | script::Phase::Move) {
            Some(submitted)
        } else {
            None
        };
        if s.phase == script::Phase::Leave {
            file.flush().map_err(|_| "Cannot flush completed stroke")?;
            println!("Stroke {} completed; {:.0} ms elapsed", s.stroke, elapsed);
        }
    }
    drop(injector);
    file.flush()
        .map_err(|_| "Cannot flush completed command journal")?;
    writeln!(receipt, "{{\"schema\":1,\"completed\":true,\"samples\":{},\"hwnd\":{},\"pid\":{},\"dpi\":{},\"mode\":\"{}\"}}", sample_count, baseline.hwnd, baseline.pid, baseline.dpi, args[5]).map_err(|_| "Cannot write completion receipt")?;
    println!(
        "Injection API calls completed. Receiver analysis is required; this alone is not acceptance."
    );
    Ok(())
}

#[cfg(windows)]
fn guard_trials(args: &[String]) -> Result<(), String> {
    if args.len() != 7 || args[5] != "--owner-ready" {
        return Err("Usage: --guard-trials <target-hwnd> <target-pid> <sink-hwnd> <sink-pid> --owner-ready <new.csv>".into());
    }
    let value = |i: usize| {
        args[i]
            .parse::<usize>()
            .map_err(|_| "Invalid target identity")
    };
    let target_hwnd = value(1)?;
    let target_pid = u32::try_from(value(2)?).map_err(|_| "PID overflow")?;
    let sink_hwnd = value(3)?;
    let sink_pid = u32::try_from(value(4)?).map_err(|_| "PID overflow")?;
    if target_hwnd == sink_hwnd || target_pid == sink_pid {
        return Err("Two distinct harness processes required".into());
    }
    platform::initialize_dpi()?;
    platform::require_harness(target_hwnd, target_pid)?;
    platform::require_harness(sink_hwnd, sink_pid)?;
    let original = platform::snapshot(target_hwnd, target_pid)?;
    let sink = platform::snapshot(sink_hwnd, sink_pid)?;
    platform::request_foreground_handoff(original)?;
    platform::request_foreground_handoff(sink)?;
    let _priority = platform::input_priority()?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[6])
        .map_err(|_| "Choose a new trial output")?;
    writeln!(
        file,
        "trial,mutation,baseline_sent,changed_attempt_blocked,resume_attempt_blocked"
    )
    .map_err(|_| "Cannot write trials")?;
    println!(
        "100 live guard trials: leave mouse/keyboard alone; only the two named harness windows are manipulated."
    );
    let outcome = (|| -> Result<(), String> {
        for trial in 0..100 {
            platform::restore_harness(original)?;
            platform::request_foreground_handoff(sink)?;
            platform::focus_harness(original)?;
            std::thread::sleep(Duration::from_millis(60));
            let baseline = platform::snapshot(target_hwnd, target_pid)?;
            let mut injector = platform::Injector::arm(baseline, u64::from(trial + 1), false)?;
            let samples = script::generate(baseline.client, false)?;
            let down = samples
                .iter()
                .find(|s| s.phase == script::Phase::Down)
                .copied()
                .ok_or("Missing down sample")?;
            if platform::native_down_count(baseline)? != trial as usize {
                return Err("Unexpected native contact count before trial".into());
            }
            injector.send(down)?;
            let delivery = Instant::now();
            loop {
                let count = platform::native_down_count(baseline)?;
                if count == trial as usize + 1 {
                    break;
                }
                if count > trial as usize + 1 || delivery.elapsed() > Duration::from_millis(250) {
                    return Err("Native baseline DOWN was not acknowledged within 250 ms".into());
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            platform::mutate_harness(baseline, sink, trial % 4)?;
            let update = script::Sample {
                phase: script::Phase::Move,
                ..down
            };
            // Only the intended target mutation counts. A scheduler timeout or
            // arbitrary API failure must not be mistaken for a successful guard trial.
            let expected = match trial % 4 {
                0 | 1 => vw_pen_probe::Stop::TargetChanged.to_string(),
                2 => "Target is absent, hidden, disabled, minimized or not a root window".into(),
                _ => vw_pen_probe::Stop::ForegroundChanged.to_string(),
            };
            let blocked = injector.send(update) == Err(expected);
            platform::restore_harness(original)?;
            platform::request_foreground_handoff(sink)?;
            platform::focus_harness(original)?;
            let latched = injector.send(update) == Err(vw_pen_probe::Stop::Suspended.to_string());
            drop(injector);
            writeln!(
                file,
                "{trial},{},true,{blocked},{latched}",
                ["move", "resize", "minimize", "focus"][(trial % 4) as usize]
            )
            .and_then(|_| file.flush())
            .map_err(|_| "Cannot persist trial result")?;
            if !blocked || !latched {
                return Err("Guard trial failed; preserve both receiver journals".into());
            }
            if trial % 10 == 9 {
                println!(
                    "Guard trials: {}/100; sink must still be checked for stray delivery",
                    trial + 1
                );
            }
        }
        Ok(())
    })();
    // Restore only while the original window/process still exists, even on failure.
    // Cleanup verifies identity even when the target is minimized. Do not hide a prior failure.
    let restored = platform::restore_harness(original);
    if outcome.is_ok() {
        restored?;
    }
    outcome
}
#[cfg(not(windows))]
fn run() -> Result<(), String> {
    Err("Windows desktop required".into())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("pen-inject: {e}");
        std::process::exit(1);
    }
}
