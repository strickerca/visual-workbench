fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let start: u64 = args.next().as_deref().unwrap_or("0").parse()?;
    let count: u64 = args.next().as_deref().unwrap_or("10000").parse()?;
    if count == 0 || count > 100_000 || args.next().is_some() {
        return Err("usage: vw-sim [first-seed] [count:1..100000]".into());
    }
    let mut duplicates = 0;
    let mut dropped = 0;
    for offset in 0..count {
        let seed = start.checked_add(offset).ok_or("seed range overflow")?;
        let report = vw_sim::run(seed).map_err(|error| format!("seed {seed}: {error}"))?;
        duplicates += report.duplicate_receipts;
        dropped += report.dropped;
        if (offset + 1) % 100 == 0 {
            eprintln!(
                "simulation progress: {}/{} seeds verified",
                offset + 1,
                count
            );
        }
    }
    println!(
        "PASS seeds={count} first_seed={start} duplicate_receipts={duplicates} dropped_packets={dropped}"
    );
    Ok(())
}
