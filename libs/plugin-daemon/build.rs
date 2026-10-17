fn main() {
    let source = "src/daemon/platform/unix/platform/macos_socket.c";
    println!("cargo:rerun-if-changed={source}");
    if qol_platform::Os::build_target() == Some(qol_platform::Os::Macos) {
        cc::Build::new()
            .file(source)
            .warnings_into_errors(true)
            .compile("qol_daemon_socket");
        println!("cargo:rustc-link-lib=proc");
    }
}
