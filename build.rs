// build.rs — copie fonts/*.ttf + offsets.json à CÔTÉ DU FUTUR EXE
// (target/<profile>/) pour que cs2-esp.exe trouve la police Space Grotesk
// et les offsets sans manipulation manuelle après `cargo build --release`.
//
// OUT_DIR = target/<profile>/build/cs2-esp-<hash>/out
//   → ancestors().nth(3) = target/<profile>  (là où cargo pose l'exe)
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("remonte à target/<profile>")
        .to_path_buf();
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    // fonts/ → target/<profile>/fonts/
    let fonts_src = root.join("fonts");
    if fonts_src.is_dir() {
        let dst = profile_dir.join("fonts");
        if std::fs::create_dir_all(&dst).is_ok() {
            let mut copied = 0usize;
            if let Ok(rd) = std::fs::read_dir(&fonts_src) {
                for e in rd.flatten() {
                    if e.path().extension().map(|x| x == "ttf").unwrap_or(false) {
                        if std::fs::copy(e.path(), dst.join(e.file_name())).is_ok() {
                            copied += 1;
                        }
                    }
                }
            }
            println!("cargo:warning=[build] {copied} polices copiées vers {}", dst.display());
        }
    } else {
        println!("cargo:warning=[build] fonts/ absent — l'exe utilisera la police système (Segoe UI)");
    }

    // offsets.json → target/<profile>/offsets.json
    let oj = root.join("offsets.json");
    if oj.exists() {
        match std::fs::copy(&oj, profile_dir.join("offsets.json")) {
            Ok(_) => println!("cargo:warning=[build] offsets.json copié vers {}", profile_dir.display()),
            Err(e) => println!("cargo:warning=[build] offsets.json non copié: {e}"),
        }
    }

    println!("cargo:rerun-if-changed=fonts/");
    println!("cargo:rerun-if-changed=offsets.json");
}
