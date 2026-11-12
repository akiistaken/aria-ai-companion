//! Build-script helpers shared by the workspace's binaries.

/// Links with the rust-lld that ships in the GNU toolchain instead of MinGW's GNU ld, which is
/// several times slower on binaries this size.
///
/// It is passed as a link arg of the calling package only, so turning it on or off relinks that
/// binary without rebuilding any dependency (a `rustflags` change would rebuild all of Bevy and
/// Slint). `ARIA_GNU_LD=1` links with the system ld again.
pub fn use_rust_lld() {
    println!("cargo:rerun-if-env-changed=ARIA_GNU_LD");
    if std::env::var_os("ARIA_GNU_LD").is_some() {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu")
        || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
    {
        return;
    }
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let Ok(out) = std::process::Command::new(rustc).args(["--print", "sysroot"]).output() else {
        return;
    };
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let target = std::env::var("TARGET").unwrap_or_default();
    let gcc_ld = std::path::Path::new(&sysroot)
        .join("lib")
        .join("rustlib")
        .join(&target)
        .join("bin")
        .join("gcc-ld");
    if !gcc_ld.join("ld.lld.exe").is_file() {
        return;
    }
    println!("cargo:rustc-link-arg=-fuse-ld=lld");
    println!("cargo:rustc-link-arg=-B{}", gcc_ld.display());
}
