use std::fs;
use std::path::Path;

fn main() {
    #[allow(non_snake_case)]
    let LISP_FILES: Vec<&str> = vec![
        "stdlib.lisp",
        "minibuffer.lisp"
    ];

    for file in &LISP_FILES {
        println!("{}", &format!("cargo::rerun-if-changed=lisp/{file}"));
    }

    let profile_dir = std::env::var("PROFILE").unwrap();
    let manifest_dir = std::env::var("CARGO_WORKSPACE_DIR").unwrap();
    let mut target_dir = Path::new(&manifest_dir).join("target").join(profile_dir);

    match fs::create_dir(format!("{}/data", target_dir.display())) {
        Ok(()) => (),
        Err(e) => match e.kind() {
            std::io::ErrorKind::AlreadyExists => (),
            _ => {
                panic!(
                    "{} Failed to create the lisp files directory /data",
                    target_dir.display()
                );
            }
        },
    }
    match fs::create_dir(format!("{}/data/lisp", target_dir.display())) {
        Ok(()) => (),
        Err(e) => match e.kind() {
            std::io::ErrorKind::AlreadyExists => (),
            _ => {
                panic!(
                    "{} Failed to create the lisp files directory /data/lisp",
                    target_dir.display()
                );
            }
        },
    }

    target_dir = target_dir.join("data").join("lisp");

    for file in LISP_FILES {
        fs::copy(
            format!("lisp/{file}"),
            format!("{}/{file}", &target_dir.display()),
        )
        .expect(&format!("Failed to copy {file}"));
    }
}
