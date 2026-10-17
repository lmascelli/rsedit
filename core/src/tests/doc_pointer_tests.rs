//! Pointers from the code to the developer manual.
//!
//! A module's documentation or a comment says where its design is explained
//! as `Design: doc/FILE.typ, <label>.` (see doc/documentazione.typ). A label
//! that is renamed or removed leaves the pointer pointing nowhere, which only
//! a test notices.
#[cfg(test)]
mod tests {
    use regex::Regex;
    use std::path::{Path, PathBuf};

    /// The pointers in TEXT, as (document, label).
    fn pointers(text: &str) -> Vec<(String, String)> {
        let pattern = Regex::new(r"doc/([a-z0-9-]+\.typ), <([a-z0-9-]+)>").expect("the pattern");
        pattern
            .captures_iter(text)
            .map(|found| (found[1].to_string(), found[2].to_string()))
            .collect()
    }

    /// The files under DIR with EXTENSION, outside the test directories.
    fn sources(dir: &Path, extension: &str, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()))
        {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "tests") {
                    sources(&path, extension, into);
                }
            } else if path.extension().is_some_and(|found| found == extension) {
                into.push(path);
            }
        }
    }

    #[test]
    fn a_pointer_is_read_as_its_document_and_label() {
        assert_eq!(
            pointers("//! Design: doc/architettura-editor.typ, <sfondo>."),
            vec![("architettura-editor.typ".to_string(), "sfondo".to_string())]
        );
        assert!(pointers(";; see doc/workmanager.typ").is_empty());
    }

    #[test]
    fn every_pointer_from_the_code_names_a_label_the_manual_has() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        sources(&root.join("core").join("src"), "rs", &mut files);
        sources(&root.join("src"), "rs", &mut files);
        sources(&root.join("core").join("lisp"), "lisp", &mut files);
        assert!(
            files.len() > 100,
            "the sources are not being found: {}",
            files.len()
        );
        let mut broken = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|why| panic!("{}: {why}", file.display()));
            for (document, label) in pointers(&text) {
                let found = std::fs::read_to_string(root.join("doc").join(&document))
                    .is_ok_and(|manual| manual.contains(&format!("<{label}>")));
                if !found {
                    broken.push(format!("{}: doc/{document}, <{label}>", file.display()));
                }
            }
        }
        assert!(
            broken.is_empty(),
            "these point at labels the manual does not have:\n  {}",
            broken.join("\n  ")
        );
    }
}
