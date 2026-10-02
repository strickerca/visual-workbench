fn main() {
    println!(
        "Visual Workbench toolchain smoke ABI v{} ({}/{})",
        vw_hello::vw_hello_toolchain_smoke_version(),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
}
