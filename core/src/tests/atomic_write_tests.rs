//! Writing a file whole or not at all: beside it, then renamed over it, with
//! its permissions; and in place where a rename would change what the file
//! is -- a symbolic link, a file with other names, one owned by somebody else.
//!
//! Design: `doc/architettura-editor.typ, <file-su-disco>`.
#[cfg(test)]
mod tests {
    use crate::buffer::disk::write_atomically;
    use std::path::{Path, PathBuf};

    /// A directory of this test's own, removed when it goes.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rsedit-atomic-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// What the directory holds, by name, sorted.
        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .expect("listing the sandbox")
                .map(|entry| {
                    entry
                        .expect("an entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("reading a file")
    }

    #[test]
    fn a_new_file_is_made() {
        let sandbox = Sandbox::new("new");
        write_atomically(sandbox.path("new.txt"), b"hello").expect("writing");
        assert_eq!(read(&sandbox.path("new.txt")), "hello");
        assert_eq!(sandbox.names(), ["new.txt"]);
    }

    #[test]
    fn an_old_file_is_replaced_and_nothing_is_left_beside_it() {
        let sandbox = Sandbox::new("replaced");
        std::fs::write(sandbox.path("file.txt"), "a much longer old text").expect("old");
        write_atomically(sandbox.path("file.txt"), b"new").expect("writing");
        assert_eq!(read(&sandbox.path("file.txt")), "new");
        assert_eq!(sandbox.names(), ["file.txt"], "no temporary file is left");
    }

    #[test]
    fn a_failed_rename_leaves_no_temporary_behind() {
        // A directory where the file should be: the new file is written, and
        // cannot be renamed over it.
        let sandbox = Sandbox::new("failed");
        std::fs::create_dir(sandbox.path("taken")).expect("a directory");
        std::fs::write(sandbox.path("taken/inside"), "x").expect("a file in it");
        assert!(write_atomically(sandbox.path("taken"), b"new").is_err());
        assert_eq!(sandbox.names(), ["taken"]);
        assert_eq!(read(&sandbox.path("taken/inside")), "x");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_a_new_one_with_the_old_ones_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let sandbox = Sandbox::new("permissions");
        let path = sandbox.path("script.sh");
        std::fs::write(&path, "old").expect("old");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).expect("chmod");
        let before = std::fs::metadata(&path).expect("stat").ino();

        write_atomically(&path, b"new").expect("writing");

        let after = std::fs::metadata(&path).expect("stat");
        assert_ne!(after.ino(), before, "renamed over, not written in place");
        assert_eq!(after.permissions().mode() & 0o777, 0o750);
        assert_eq!(read(&path), "new");
    }

    #[cfg(unix)]
    #[test]
    fn through_a_symbolic_link_the_file_it_names_is_written_and_the_link_stays() {
        let sandbox = Sandbox::new("symlink");
        std::fs::write(sandbox.path("real.txt"), "old").expect("old");
        std::os::unix::fs::symlink("real.txt", sandbox.path("link.txt")).expect("link");

        write_atomically(sandbox.path("link.txt"), b"new").expect("writing");

        let link = std::fs::symlink_metadata(sandbox.path("link.txt")).expect("lstat");
        assert!(link.file_type().is_symlink(), "still a link");
        assert_eq!(read(&sandbox.path("real.txt")), "new");
        assert_eq!(sandbox.names(), ["link.txt", "real.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_nothing_makes_the_file_it_names() {
        let sandbox = Sandbox::new("dangling");
        std::os::unix::fs::symlink("later.txt", sandbox.path("link.txt")).expect("link");

        write_atomically(sandbox.path("link.txt"), b"new").expect("writing");

        assert_eq!(read(&sandbox.path("later.txt")), "new");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_with_other_names_is_written_in_place_so_all_see_it() {
        use std::os::unix::fs::MetadataExt;
        let sandbox = Sandbox::new("hardlink");
        std::fs::write(sandbox.path("one.txt"), "old").expect("old");
        std::fs::hard_link(sandbox.path("one.txt"), sandbox.path("two.txt")).expect("link");
        let before = std::fs::metadata(sandbox.path("one.txt"))
            .expect("stat")
            .ino();

        write_atomically(sandbox.path("one.txt"), b"new").expect("writing");

        assert_eq!(read(&sandbox.path("two.txt")), "new");
        assert_eq!(
            std::fs::metadata(sandbox.path("one.txt"))
                .expect("stat")
                .ino(),
            before
        );
    }

    /// Only where the test may give a file away, as root may: a new file
    /// would be the writer's, so the file is written in place and stays its
    /// owner's.
    #[cfg(unix)]
    #[test]
    fn a_file_owned_by_somebody_else_keeps_its_owner() {
        use std::os::unix::fs::MetadataExt;
        let sandbox = Sandbox::new("owner");
        let path = sandbox.path("theirs.txt");
        std::fs::write(&path, "old").expect("old");
        if std::os::unix::fs::chown(&path, Some(54_321), Some(54_321)).is_err() {
            return;
        }

        write_atomically(&path, b"new").expect("writing");

        let after = std::fs::metadata(&path).expect("stat");
        assert_eq!((after.uid(), after.gid()), (54_321, 54_321));
        assert_eq!(read(&path), "new");
    }

    /// The editor's own save goes this way: the permissions of a file stay
    /// what they were.
    #[cfg(unix)]
    #[test]
    fn saving_from_the_editor_keeps_the_files_permissions() {
        use crate::buffer::gap_buffer::GapBuffer;
        use crate::editor::create_global_env;
        use risp::{Parser, eval};
        use std::os::unix::fs::PermissionsExt;
        let sandbox = Sandbox::new("editor");
        let path = sandbox.path("run.sh");
        std::fs::write(&path, "echo old\n").expect("old");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        let src = format!(
            r#"(progn (find-file "{}") (end-of-buffer) (insert "echo new\n") (save-buffer))"#,
            path.display()
        );
        let ast = Parser::new(&src).next().expect("the test must parse");
        eval(&ast, env.clone(), &ctx).expect("saving");

        assert_eq!(read(&path), "echo old\necho new\n");
        assert_eq!(
            std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(sandbox.names(), ["run.sh"]);
    }
}
