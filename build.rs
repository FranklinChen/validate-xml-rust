fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        return;
    }

    let library = pkg_config::Config::new()
        .cargo_metadata(true)
        .probe("libxml-2.0")
        .expect("libxml2 development files must be discoverable through pkg-config");
    println!("cargo:rustc-env=LIBXML2_BUILD_VERSION={}", library.version);
}
