use image::{Rgba, RgbaImage};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use vw_ai_spike::{
    Error, Result,
    budget::Reservation,
    config::ProviderConfig,
    geometry::Rect,
    pixels,
    request::{MockTransport, Prepared, Transport},
    sha256,
};

const HELP: &str = "ai-spike --mode prepare|mock|send --source IMAGE --mask ALPHA_PNG|--rect X,Y,W,H --prompt-file TEXT --out NEW_DIRECTORY [--config JSON] [--feather 8] [--assume-srgb]\nOffline fixtures: ai-spike --mode mock --synthetic-case color|object|text|border|small --out NEW_DIRECTORY\nLive only: repeat prepared inputs with --mode send --confirm REQUEST_ID; requires configured token_estimate and Credential Manager VisualWorkbench/openai, username openai. No key argument or environment variable is accepted.\nPersistent live ledger: LOCALAPPDATA/VisualWorkbench/ai-spike-budget.json; no command-line override. Never delete or rotate it to evade the $2 spike reservation guard.\nAll output folders contain private pixels/prompt data. Keep them outside Git, independently verify, then dispose task verification files.";

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit {
        return Err(Error::Limit("input file"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Limit("input file"));
    }
    Ok(bytes)
}

fn arguments() -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(key) = args.next() {
        if key == "--help" {
            println!("{HELP}");
            return Ok(BTreeMap::new());
        }
        if ![
            "--mode",
            "--source",
            "--mask",
            "--rect",
            "--prompt-file",
            "--out",
            "--config",
            "--feather",
            "--assume-srgb",
            "--synthetic-case",
            "--confirm",
        ]
        .contains(&key.as_str())
        {
            return Err(Error::Invalid("unknown command option"));
        }
        let value = if key == "--assume-srgb" {
            "true".into()
        } else {
            args.next().ok_or(Error::Invalid("missing option value"))?
        };
        if values.insert(key, value).is_some() {
            return Err(Error::Invalid("duplicate command option"));
        }
    }
    Ok(values)
}

fn required<'a>(args: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    args.get(key)
        .map(String::as_str)
        .ok_or(Error::Invalid("required command option missing"))
}

fn parse_rect(value: &str) -> Result<Rect> {
    let parts: Vec<i32> = value
        .split(',')
        .map(|part| {
            part.parse()
                .map_err(|_| Error::Invalid("rectangle integers"))
        })
        .collect::<Result<_>>()?;
    if parts.len() != 4 || parts[0] < 0 || parts[1] < 0 || parts[2] <= 0 || parts[3] <= 0 {
        return Err(Error::Invalid("rectangle"));
    }
    Ok(Rect {
        x: parts[0],
        y: parts[1],
        width: parts[2] as u32,
        height: parts[3] as u32,
    })
}

fn synthetic(case: &str) -> Result<(Vec<u8>, Rect, String)> {
    if !["color", "object", "text", "border", "small"].contains(&case) {
        return Err(Error::Invalid("synthetic case"));
    }
    let image = RgbaImage::from_fn(384, 256, |x, y| {
        if (70..180).contains(&x) && (50..125).contains(&y) {
            if (x / 7 + y / 9).is_multiple_of(3) {
                Rgba([30, 40, 50, 255])
            } else {
                Rgba([235, 225, 210, 255])
            }
        } else {
            Rgba([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8, 255])
        }
    });
    let rect = match case {
        "border" => Rect {
            x: 0,
            y: 100,
            width: 17,
            height: 90,
        },
        "small" => Rect {
            x: 180,
            y: 120,
            width: 9,
            height: 11,
        },
        _ => Rect {
            x: 80,
            y: 60,
            width: 64,
            height: 55,
        },
    };
    Ok((
        pixels::encode_png(&image, None)?,
        rect,
        format!(
            "Synthetic offline {case} fixture; paint the change region blue. This is a compositor test, not semantic model evidence."
        ),
    ))
}

fn write_new(directory: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join(name))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn default_ledger() -> Result<PathBuf> {
    let local =
        std::env::var_os("LOCALAPPDATA").ok_or(Error::Invalid("LOCALAPPDATA is unavailable"))?;
    let parent = PathBuf::from(local).join("VisualWorkbench");
    fs::create_dir_all(&parent)?;
    Ok(parent.join("ai-spike-budget.json"))
}

fn run() -> Result<()> {
    let args = arguments()?;
    if args.is_empty() {
        return Ok(());
    }
    let mode = required(&args, "--mode")?;
    if !["prepare", "mock", "send"].contains(&mode) {
        return Err(Error::Invalid("mode"));
    }
    if mode != "send" && args.contains_key("--confirm") {
        return Err(Error::Invalid("live options are valid only in send mode"));
    }
    let output = Path::new(required(&args, "--out")?);
    if output.exists() {
        return Err(Error::Invalid("output directory already exists"));
    }
    let config = match args.get("--config") {
        Some(path) => ProviderConfig::from_json(&read_bounded(Path::new(path), 65_536)?)?,
        None => ProviderConfig::bundled()?,
    };
    let radius = args
        .get("--feather")
        .map(|s| {
            s.parse::<u32>()
                .map_err(|_| Error::Invalid("feather integer"))
        })
        .transpose()?
        .unwrap_or(8);
    let (original, mask, instruction) = if let Some(case) = args.get("--synthetic-case") {
        if mode == "send"
            || ["--source", "--mask", "--rect", "--prompt-file"]
                .iter()
                .any(|key| args.contains_key(*key))
        {
            return Err(Error::Invalid(
                "synthetic cases are offline-only with fixed source inputs",
            ));
        }
        let (bytes, rect, prompt) = synthetic(case)?;
        let mask = pixels::rectangle_mask(384, 256, rect)?;
        (bytes, mask, prompt)
    } else {
        let original = read_bounded(Path::new(required(&args, "--source")?), 100_000_000)?;
        let source = pixels::decode(&original, args.contains_key("--assume-srgb"))?;
        let mask = match (args.get("--mask"), args.get("--rect")) {
            (Some(path), None) => pixels::decode_mask(
                &read_bounded(Path::new(path), 100_000_000)?,
                source.pixels.width(),
                source.pixels.height(),
            )?,
            (None, Some(rect)) => pixels::rectangle_mask(
                source.pixels.width(),
                source.pixels.height(),
                parse_rect(rect)?,
            )?,
            _ => return Err(Error::Invalid("exactly one mask or rectangle is required")),
        };
        let prompt = String::from_utf8(read_bounded(
            Path::new(required(&args, "--prompt-file")?),
            128_000,
        )?)
        .map_err(|_| Error::Invalid("UTF-8 prompt required"))?;
        (original, mask, prompt)
    };
    let source_hash = sha256(&original);
    let source = pixels::decode(&original, args.contains_key("--assume-srgb"))?;
    let prepared = Prepared::new(
        source,
        source_hash.clone(),
        &[mask],
        &instruction,
        radius,
        config,
    )?;
    let multipart = prepared.multipart()?;
    drop(multipart); // Validates wire preparation offline without retaining another image copy.
    println!(
        "request_id={} estimated_microusd={:?} mode={mode}",
        prepared.request_id, prepared.description.estimated_microusd
    );
    if let Some(path) = args.get("--source")
        && sha256(&read_bounded(Path::new(path), 100_000_000)?) != source_hash
    {
        return Err(Error::Invalid("original source changed concurrently"));
    }
    // Reserve a new private destination and persist the exact request before any
    // paid action. Failed runs retain an incomplete bundle without result.json.
    fs::create_dir(output)?;
    write_new(output, "original.input", &original)?;
    write_new(
        output,
        "source.png",
        &pixels::encode_png(&prepared.source.pixels, prepared.source.icc.as_deref())?,
    )?;
    write_new(
        output,
        "mask.png",
        &pixels::mask_png(
            &prepared.mask,
            prepared.source.pixels.width(),
            prepared.source.pixels.height(),
        )?,
    )?;
    write_new(output, "request-image.png", &prepared.image_png)?;
    write_new(output, "request-mask.png", &prepared.mask_png)?;
    write_new(
        output,
        "prepared.json",
        &serde_json::to_vec_pretty(
            &serde_json::json!({"request_id":prepared.request_id,"description":prepared.description}),
        )?,
    )?;
    let completed = match mode {
        "prepare" => None,
        "mock" => Some(
            prepared.finish(
                MockTransport {
                    color: [35, 95, 225, 255],
                }
                .edit(&prepared)?,
            )?,
        ),
        "send" => {
            let confirmation = required(&args, "--confirm")?;
            let ledger = default_ledger()?;
            let day = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| Error::Invalid("system clock"))?
                .as_secs()
                / 86_400;
            let mut reservation = Reservation::begin(
                &ledger,
                &prepared.description.provider,
                &prepared.request_id,
                confirmation,
                day,
            )?;
            if reservation.daily_soft_budget_exceeded {
                eprintln!(
                    "Daily soft budget warning: this confirmed reservation exceeds the configured daily estimate."
                );
            }
            let response = match vw_ai_spike::platform::send_confirmed(
                &prepared,
                confirmation,
                &mut reservation,
            ) {
                Ok(response) => response,
                Err(
                    error @ (Error::Credential | Error::UnsupportedPlatform | Error::Confirmation),
                ) => {
                    reservation.not_sent()?;
                    return Err(error);
                }
                Err(error) => return Err(error), // Durable reservation remains unresolved; no automatic retry.
            };
            let actual = response
                .tokens
                .map(|tokens| prepared.description.provider.prices.cost(tokens))
                .transpose()?;
            reservation.settle(actual)?;
            Some(prepared.finish(response)?)
        }
        _ => return Err(Error::Invalid("mode")),
    };
    if let Some(path) = args.get("--source")
        && sha256(&read_bounded(Path::new(path), 100_000_000)?) != source_hash
    {
        return Err(Error::Invalid("original source changed concurrently"));
    }
    if let Some(result) = completed {
        write_new(output, "provider-result.png", &result.provider_image)?;
        write_new(
            output,
            "composite.png",
            &pixels::encode_png(&result.composite, prepared.source.icc.as_deref())?,
        )?;
        write_new(
            output,
            "proof.json",
            &serde_json::to_vec_pretty(&result.proof)?,
        )?;
        write_new(
            output,
            "result.json",
            &serde_json::to_vec_pretty(&serde_json::json!({
                "schema":1,"status":"composite_exterior_verified","provider_mode":if result.mock {"mock"} else {"live"},
                "live_acceptance":false,"request_id":prepared.request_id,"changed_outside":result.proof.changed_outside,
                "local_processing_ms":result.local_ms,"provider_ms":result.provider_ms,
                "estimated_microusd":prepared.description.estimated_microusd,"actual_microusd":result.actual_microusd,
                "spend_unresolved":!result.mock && result.actual_microusd.is_none(),"original_file_unchanged":true
            }))?,
        )?;
        println!(
            "composite exterior verified; changed_outside=0; independent verification remains required"
        );
    } else {
        write_new(output, "result.json", b"{\"schema\":1,\"status\":\"prepared_only\",\"sent\":false,\"credentials_read\":false}\n")?;
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
