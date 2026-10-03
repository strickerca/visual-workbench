#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    // No printing, shell, settings file or general command execution surface.
    std::process::exit(vw_host::connection_assist_helper_main());
}
