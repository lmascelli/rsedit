//! Pointers from the code, the tests and the Lisp modules to the developer
//! manual, written ``Design: `doc/FILE.typ, <label>`.``, the header every
//! shipped Lisp module begins with, and the manual's own pointers: to the
//! problem report's cards, and to the files its excerpts come from.
//!
//! Design: `doc/architettura-editor.typ, <test-documentazione>`.
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

    /// Adds the files under DIR with EXTENSION to INTO.
    fn sources(dir: &Path, extension: &str, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()))
        {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                sources(&path, extension, into);
            } else if path.extension().is_some_and(|found| found == extension) {
                into.push(path);
            }
        }
    }

    #[test]
    fn a_pointer_is_read_as_its_document_and_label() {
        assert_eq!(
            pointers("//! Design: `doc/architettura-editor.typ, <sfondo>`."),
            vec![("architettura-editor.typ".to_string(), "sfondo".to_string())]
        );
        assert!(pointers(";; see doc/rapporto-problemi.typ").is_empty());
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

    /// A shipped module begins with `;;; NAME --- `, NAME its file's, and its
    /// header names its manual page and the manual's design.
    #[test]
    fn every_shipped_module_has_the_header() {
        let lisp = Path::new(env!("CARGO_MANIFEST_DIR")).join("lisp");
        let mut files = Vec::new();
        sources(&lisp, "lisp", &mut files);
        assert!(files.len() > 20, "the modules are not being found");
        let mut wrong = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|why| panic!("{}: {why}", file.display()));
            let name = file
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("");
            let header: Vec<&str> = text
                .lines()
                .take_while(|line| line.starts_with(";;;"))
                .collect();
            let first = header.first().copied().unwrap_or("");
            let body = header.join("\n");
            if !first.starts_with(&format!(";;; {name} --- "))
                || !body.contains("User manual:")
                || pointers(&body).is_empty()
            {
                wrong.push(file.display().to_string());
            }
        }
        assert!(
            wrong.is_empty(),
            "these modules lack the header, its page or its Design:\n  {}",
            wrong.join("\n  ")
        );
    }

    /// Returns the text of the document NAME in `doc/`.
    fn document(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("doc")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|why| panic!("{}: {why}", path.display()))
    }

    /// Every `#rapporto("ID")` in the manual names a card the report has, and
    /// one still open: a corrected defect leaves no pointer behind.
    #[test]
    fn every_card_the_manual_points_at_is_open() {
        let report = document("rapporto-problemi.typ");
        let card = Regex::new(r#"id: "([A-Z]+-[0-9]+)""#).expect("the pattern");
        let state = Regex::new(r#"stato: "([a-z]+)""#).expect("the pattern");
        let starts: Vec<(usize, String)> = card
            .captures_iter(&report)
            .map(|found| (found.get(0).expect("a match").start(), found[1].to_string()))
            .collect();
        let mut open = Vec::new();
        for (index, (start, id)) in starts.iter().enumerate() {
            let end = starts.get(index + 1).map_or(report.len(), |next| next.0);
            if state
                .captures(&report[*start..end])
                .is_some_and(|found| &found[1] == "aperto")
            {
                open.push(id.clone());
            }
        }
        assert!(open.len() > 20, "the cards are not being read");
        let manual = document("architettura-editor.typ");
        let pointer = Regex::new(r#"#rapporto\("([A-Z]+-[0-9]+)"\)"#).expect("the pattern");
        let wrong: Vec<String> = pointer
            .captures_iter(&manual)
            .map(|found| found[1].to_string())
            .filter(|id| !open.contains(id))
            .collect();
        assert!(
            wrong.is_empty(),
            "the manual points at cards that are not open: {wrong:?}"
        );
    }

    /// Every `#estratto("PATH")` in the manual comes from a file that exists,
    /// PATH relative to `core/src/`.
    #[test]
    fn every_excerpt_comes_from_a_file_that_exists() {
        let manual = document("architettura-editor.typ");
        let excerpt = Regex::new(r#"#estratto\("([^"]+)"\)"#).expect("the pattern");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let paths: Vec<String> = excerpt
            .captures_iter(&manual)
            .map(|found| found[1].to_string())
            .collect();
        assert!(paths.len() > 10, "the excerpts are not being read");
        let missing: Vec<&String> = paths
            .iter()
            .filter(|path| !source.join(path).is_file())
            .collect();
        assert!(
            missing.is_empty(),
            "the manual quotes files that are not there: {missing:?}"
        );
    }
}
