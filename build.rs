use std::fmt::Write as _;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/voice");
    println!("cargo:rerun-if-changed=assets/fonts");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Space Detective");
        res.set("FileDescription", "Space Detective");
        res.set("OriginalFilename", "space-detective.exe");
        res.compile().expect("embed Windows icon and version info");
    }

    // Embed whichever voice clips and fonts exist, so the app builds before they're all generated.
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    std::fs::write(out.join("voice_clips.rs"), embed_list("CLIPS", "assets/voice", "ogg")).unwrap();
    std::fs::write(out.join("fonts.rs"), embed_list("FONT_FILES", "assets/fonts", "ttf")).unwrap();
}

/// `pub static NAME: &[(&str, &[u8])]` of every `*.ext` in `dir`, keyed by file stem.
fn embed_list(name: &str, dir: &str, ext: &str) -> String {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    files.sort();
    let mut code = format!("pub static {name}: &[(&str, &[u8])] = &[\n");
    for path in files {
        let stem = path.file_stem().unwrap().to_string_lossy();
        let file = path.file_name().unwrap().to_string_lossy();
        writeln!(code, "    ({stem:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{dir}/{file}\"))),").unwrap();
    }
    code.push_str("];\n");
    code
}
