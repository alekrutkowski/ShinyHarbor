fn main() {
    println!("cargo:rerun-if-changed=assets/brand/shinyharbor.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/brand/shinyharbor.ico")
            .set("ProductName", "ShinyHarbor")
            .set("FileDescription", "ShinyHarbor R application server")
            .compile()
            .expect("compile Windows icon and version resources");
    }
}
