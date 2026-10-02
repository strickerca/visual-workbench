//! Bounded measurement protocol, not the product's multiplexing or pairing layer.
use serde::Serialize;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_MESSAGE: usize = 1024 * 1024;
pub const MAX_BULK: u64 = 512 * 1024 * 1024;
pub const IO_DEADLINE: Duration = Duration::from_secs(15);
pub const BLOCK: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid benchmark configuration or protocol frame")]
    Invalid,
    #[error("payload mismatch")]
    Corrupt,
    #[error("bounded operation timed out")]
    Timeout,
    #[error("network or file operation failed")]
    Io(#[from] std::io::Error),
    #[error("QUIC operation failed")]
    Quic,
    #[error("certificate validation/configuration failed")]
    Certificate,
    #[error("report serialization failed")]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

pub async fn bounded<F: std::future::Future>(future: F) -> Result<F::Output> {
    tokio::time::timeout(IO_DEADLINE, future)
        .await
        .map_err(|_| Error::Timeout)
}

#[derive(Debug, Serialize)]
pub struct Stats {
    pub count: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub jitter_mean_absolute_successive_delta_ms: f64,
}
pub fn summarize(samples: &[f64]) -> Result<Stats> {
    if samples.is_empty() || samples.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(Error::Invalid);
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let quantile = |p: f64| sorted[((p * sorted.len() as f64).ceil() as usize).saturating_sub(1)];
    let jitter = if samples.len() > 1 {
        samples.windows(2).map(|s| (s[1] - s[0]).abs()).sum::<f64>() / (samples.len() - 1) as f64
    } else {
        0.0
    };
    Ok(Stats {
        count: samples.len(),
        min_ms: sorted[0],
        p50_ms: quantile(0.5),
        p95_ms: quantile(0.95),
        p99_ms: quantile(0.99),
        max_ms: sorted[sorted.len() - 1],
        jitter_mean_absolute_successive_delta_ms: jitter,
    })
}

/// One peer, bounded frames, deterministic synthetic bytes only. Empty op stream
/// is an error; explicit DONE and ACK establish successful completion.
pub async fn serve_stream<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut input: R,
    mut output: W,
) -> Result<()> {
    let mut processed = 0u64;
    loop {
        let op = bounded(input.read_u8()).await??;
        match op {
            1 => {
                let size = bounded(input.read_u32()).await?? as usize;
                if size == 0 || size > MAX_MESSAGE {
                    return Err(Error::Invalid);
                }
                processed = processed.checked_add(size as u64).ok_or(Error::Invalid)?;
                if processed > 16 * 1024 * 1024 * 1024 {
                    return Err(Error::Invalid);
                }
                let mut payload = vec![0; size];
                bounded(input.read_exact(&mut payload)).await??;
                bounded(output.write_all(&payload)).await??;
                bounded(output.flush()).await??;
            }
            2 => {
                let mut remaining = bounded(input.read_u64()).await??;
                if remaining == 0 || remaining > MAX_BULK {
                    return Err(Error::Invalid);
                }
                let mut payload = vec![0; BLOCK];
                while remaining > 0 {
                    let count = remaining.min(BLOCK as u64) as usize;
                    bounded(input.read_exact(&mut payload[..count])).await??;
                    if payload[..count]
                        .iter()
                        .enumerate()
                        .any(|(i, byte)| *byte != i as u8)
                    {
                        return Err(Error::Corrupt);
                    }
                    remaining -= count as u64;
                }
                bounded(output.write_u8(2)).await??;
                bounded(output.flush()).await??;
            }
            3 => {
                bounded(output.write_u8(3)).await??;
                bounded(output.flush()).await??;
                return Ok(());
            }
            _ => return Err(Error::Invalid),
        }
    }
}

#[derive(Serialize)]
pub struct EchoResult {
    pub bytes: usize,
    pub rtt: Stats,
    pub raw_rtt_ms: Vec<f64>,
}
#[derive(Serialize)]
pub struct StreamResult {
    pub echo: Vec<EchoResult>,
    pub bulk_bytes: u64,
    pub bulk_elapsed_ms: f64,
    pub bulk_mib_per_second: f64,
    pub bulk_direction: &'static str,
    pub payloads_verified: bool,
}
pub async fn measure_stream<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    input: &mut R,
    output: &mut W,
    iterations: usize,
    bulk: u64,
) -> Result<StreamResult> {
    if !(1..=10_000).contains(&iterations) || bulk == 0 || bulk > MAX_BULK {
        return Err(Error::Invalid);
    }
    let mut echo = Vec::new();
    for size in [64, 4096, MAX_MESSAGE] {
        let mut payload = vec![0; size];
        let mut received = vec![0; size];
        let mut samples = Vec::with_capacity(iterations);
        for sequence in 0..(iterations + 10) {
            payload.fill((sequence % 251) as u8);
            let start = std::time::Instant::now();
            bounded(output.write_u8(1)).await??;
            bounded(output.write_u32(size as u32)).await??;
            bounded(output.write_all(&payload)).await??;
            bounded(output.flush()).await??;
            bounded(input.read_exact(&mut received)).await??;
            let rtt = start.elapsed().as_secs_f64() * 1000.0;
            if received != payload {
                return Err(Error::Corrupt);
            }
            if sequence >= 10 {
                samples.push(rtt);
            }
            if sequence > 0 && sequence % 100 == 0 {
                eprintln!("echo {size} B: {sequence}/{iterations} (+10 warmup)");
            }
        }
        let rtt = summarize(&samples)?;
        echo.push(EchoResult {
            bytes: size,
            rtt,
            raw_rtt_ms: samples,
        });
        eprintln!("echo {size} B complete");
    }
    let payload: Vec<u8> = (0..BLOCK).map(|i| i as u8).collect();
    let started = std::time::Instant::now();
    bounded(output.write_u8(2)).await??;
    bounded(output.write_u64(bulk)).await??;
    let mut remaining = bulk;
    while remaining > 0 {
        let count = remaining.min(BLOCK as u64) as usize;
        bounded(output.write_all(&payload[..count])).await??;
        remaining -= count as u64;
    }
    bounded(output.flush()).await??;
    if bounded(input.read_u8()).await?? != 2 {
        return Err(Error::Corrupt);
    }
    let seconds = started.elapsed().as_secs_f64();
    if seconds <= 0.0 {
        return Err(Error::Invalid);
    }
    eprintln!("bulk complete: {} MiB verified", bulk / 1024 / 1024);
    Ok(StreamResult {
        echo,
        bulk_bytes: bulk,
        bulk_elapsed_ms: seconds * 1000.0,
        bulk_mib_per_second: bulk as f64 / 1024.0 / 1024.0 / seconds,
        bulk_direction: "client_to_server_with_verified_ack",
        payloads_verified: true,
    })
}

pub async fn finish<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    input: &mut R,
    output: &mut W,
) -> Result<()> {
    bounded(output.write_u8(3)).await??;
    bounded(output.flush()).await??;
    if bounded(input.read_u8()).await?? != 3 {
        return Err(Error::Corrupt);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantiles_use_nearest_rank_and_jitter_keeps_temporal_order() -> Result<()> {
        let result = summarize(&[4.0, 1.0, 3.0, 2.0])?;
        assert_eq!(result.p50_ms, 2.0);
        assert_eq!(result.p95_ms, 4.0);
        assert_eq!(result.jitter_mean_absolute_successive_delta_ms, 2.0);
        assert!(summarize(&[]).is_err());
        assert!(summarize(&[f64::NAN]).is_err());
        Ok(())
    }
    #[tokio::test]
    async fn overlarge_frame_rejected_before_payload_allocation() -> Result<()> {
        let (mut client, server) = tokio::io::duplex(64);
        let (read, write) = tokio::io::split(server);
        let server = tokio::spawn(serve_stream(read, write));
        client.write_u8(1).await?;
        client.write_u32((MAX_MESSAGE + 1) as u32).await?;
        assert!(matches!(
            server.await.map_err(|_| Error::Invalid)?,
            Err(Error::Invalid)
        ));
        Ok(())
    }
    #[tokio::test]
    async fn corruption_rejected_without_success_ack() -> Result<()> {
        let (mut client, server) = tokio::io::duplex(64);
        let (read, write) = tokio::io::split(server);
        let server = tokio::spawn(serve_stream(read, write));
        client.write_u8(2).await?;
        client.write_u64(1).await?;
        client.write_u8(99).await?;
        assert!(matches!(
            server.await.map_err(|_| Error::Invalid)?,
            Err(Error::Corrupt)
        ));
        Ok(())
    }
}
