use std::fs;
use std::path::Path;

fn main() {
    #[allow(non_snake_case)]
    let LISP_FILES: Vec<&str> = vec![
        "buffer-list.lisp",
        "clipboard.lisp",
        "compile.lisp",
        "commands.lisp",
        "common-keymaps.lisp",
        "completion-at-point.lisp",
        "completion.lisp",
        "debug.lisp",
        "dired.lisp",
        "electric-pair.lisp",
        "indent.lisp",
        "manpage.lisp",
        "minibuffer.lisp",
        "risp-mode.lisp",
        "rust-mode.lisp",
        "shell.lisp",
    ];

    for file in &LISP_FILES {
        println!("{}", &format!("cargo::rerun-if-changed=lisp/{file}"));
    }

    let profile_dir = std::env::var("PROFILE").unwrap();
    let manifest_dir = std::env::var("CARGO_WORKSPACE_DIR").unwrap();
    let target_dir = Path::new(&manifest_dir).join("target").join(profile_dir);

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

    let lisp_dir = target_dir.join("data").join("lisp");

    for file in LISP_FILES {
        fs::copy(
            format!("lisp/{file}"),
            format!("{}/{file}", &lisp_dir.display()),
        )
        .expect(&format!("Failed to copy {file}"));
    }

    // The manual pages, beside the modules and for the same reason: they are
    // data the editor reads at runtime, so they have to sit next to the binary
    // rather than in the source tree it was built from.
    //
    // Copied wholesale rather than from a list, unlike the Lisp above. A module
    // that is not loaded is a feature that silently does not exist, which is
    // worth a list you have to edit; a page that is not copied is a page
    // `rsedit-man' reports as missing, which says so plainly.
    let man_source = Path::new("../man");
    let man_dir = target_dir.join("data").join("man");
    if man_source.is_dir() {
        println!("cargo::rerun-if-changed=../man");
        let _ = fs::create_dir_all(&man_dir);
        for entry in fs::read_dir(man_source).expect("Failed to read the man directory") {
            let entry = entry.expect("Failed to read a man page");
            // Only the pages themselves. README.md explains the format to
            // whoever writes one and is not a page -- shipping it would put a
            // `README' in the editor's completion list for manual pages.
            let is_page = entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "txt");
            if entry.path().is_file() && is_page {
                let name = entry.file_name();
                fs::copy(entry.path(), man_dir.join(&name))
                    .unwrap_or_else(|_| panic!("Failed to copy {}", name.to_string_lossy()));
            }
        }
    }
}
