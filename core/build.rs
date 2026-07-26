// use std::fs;
use std::path::Path;

fn main() {

    let LISP_FILES: Vec<&str> = vec![
        "stdlib.lisp",
    ];

    for file in &LISP_FILES {
        println!("{}", &format!("cargo::rerun-if-changed=lisp/{file}"));
    }

    let profile_dir = std::env::var("PROFILE").unwrap();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let target_dir = Path::new(&manifest_dir)
        .join("target")
        .join(profile_dir);

        for file in LISP_FILES {
            println!("{}", &format!("cargo:warning=lisp/{file}"));
        }
}
