use std::{env, fs, path::Path};

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    fs::write(
        Path::new(&out_dir).join("style.css"),
        grass::from_path("src/style.scss", &Default::default()).unwrap(),
    )
    .unwrap();
    println!("cargo::rerun-if-changed=src/style.scss");
}
