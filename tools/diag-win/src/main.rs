mod json;
#[cfg(windows)]
mod platform;
mod png;

#[cfg(windows)]
fn run() -> Result<(), String> {
    // Set PMv2 before display, COM, screenshot or other DPI-dependent APIs.
    platform::initialize_dpi()?;
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments.as_slice() {
        [] => platform::diagnostics()?,
        [flag, output] if flag == "--out" => {
            let result = platform::diagnostics()?;
            std::fs::write(output, format!("{result}\n")).map_err(|_| "Cannot write diagnostics output")?;
            result
        }
        [flag, title, output] if flag == "--screenshot" => platform::screenshot(title, std::path::Path::new(output))?,
        _ => return Err("Usage: diag-win [--out <json>] | --screenshot <unique-window-title-substring> <outside-repository.png>".into()),
    };
    println!("{result}");
    Ok(())
}

#[cfg(not(windows))]
fn run() -> Result<(), String> {
    Err("Windows hardware diagnostics require Windows; no hardware result produced".into())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("diag-win: {error}");
        std::process::exit(1);
    }
}
