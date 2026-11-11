fn main() {
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=src/stage_bridge.js");
    slint_build::compile("ui/app.slint").expect("slint build failed");
}
