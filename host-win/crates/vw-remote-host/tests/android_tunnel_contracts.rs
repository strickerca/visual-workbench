// Compile the isolated new module's fixtures before root registers it in lib.rs.
#![allow(dead_code, unused_imports)]
#[path = "../src/android_tunnel.rs"]
mod android_tunnel;
