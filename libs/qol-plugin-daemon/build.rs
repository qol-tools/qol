fn main() {
    let source = "src/daemon/platform/unix/macos_socket.c";
    println!("cargo:rerun-if-changed={source}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file(source)
            .warnings_into_errors(true)
            .compile("qol_daemon_socket");
        println!("cargo:rustc-link-lib=proc");
    }
}
