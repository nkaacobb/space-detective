fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Space Detective");
        res.set("FileDescription", "Space Detective");
        res.set("OriginalFilename", "space-detective.exe");
        res.compile().expect("embed Windows icon and version info");
    }
}
