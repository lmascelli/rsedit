//! The coding a file is read and written in: UTF-8, UTF-8 behind its
//! signature, and Windows-1252 for every file that is not UTF-8. The table on
//! its own first, then a file through the editor: opened, saved unchanged,
//! edited, refused a character its coding lacks, converted, reverted and
//! reloaded from the disk.
//!
//! Design: `doc/architettura-editor.typ, <codifiche>`.
#[cfg(test)]
mod tests {
    use crate::buffer::coding::{Coding, decode, encode};
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    type Ctx = EditorState<GapBuffer>;

    /// Every byte once, in order: not UTF-8, since 0x80 cannot begin a
    /// character.
    fn every_byte() -> Vec<u8> {
        (0..=255).collect()
    }

    // ----------------------------------------------------------------
    // The table
    // ----------------------------------------------------------------

    #[test]
    fn utf8_is_read_as_utf8() {
        assert_eq!(
            decode("perché".as_bytes()),
            ("perché".to_string(), Coding::Utf8)
        );
        assert_eq!(decode(b"plain"), ("plain".to_string(), Coding::Utf8));
        assert_eq!(decode(b""), (String::new(), Coding::Utf8));
    }

    #[test]
    fn latin_1_is_read_as_windows_1252() {
        assert_eq!(
            decode(b"perch\xE9 cos\xEC"),
            ("perché così".to_string(), Coding::Windows1252)
        );
    }

    #[test]
    fn the_bytes_where_windows_1252_parts_from_latin_1_are_its_own() {
        assert_eq!(
            decode(b"\x80 \x93x\x94 \x85 \x99").0,
            "\u{20AC} \u{201C}x\u{201D} \u{2026} \u{2122}"
        );
        // The five it leaves undefined keep their C1 control.
        assert_eq!(
            decode(b"\x81\x8D\x8F\x90\x9D").0,
            "\u{81}\u{8D}\u{8F}\u{90}\u{9D}"
        );
    }

    #[test]
    fn every_byte_reads_as_a_character_of_its_own_and_comes_back() {
        let (text, coding) = decode(&every_byte());
        assert_eq!(coding, Coding::Windows1252);
        let distinct: std::collections::HashSet<char> = text.chars().collect();
        assert_eq!(distinct.len(), 256);
        assert_eq!(encode(&text, coding), Ok(every_byte()));
    }

    #[test]
    fn a_signature_is_taken_off_and_put_back() {
        let bytes = b"\xEF\xBB\xBFciao".to_vec();
        let (text, coding) = decode(&bytes);
        assert_eq!((text.as_str(), coding), ("ciao", Coding::Utf8WithSignature));
        assert_eq!(encode(&text, coding), Ok(bytes));
    }

    #[test]
    fn a_character_windows_1252_lacks_is_refused() {
        assert_eq!(encode("a\u{101}b", Coding::Windows1252), Err('\u{101}'));
        assert_eq!(encode("\u{80}", Coding::Windows1252), Err('\u{80}'));
        assert_eq!(encode("è€", Coding::Windows1252), Ok(b"\xE8\x80".to_vec()));
    }

    #[test]
    fn a_nul_early_on_is_the_sign_of_a_binary_file() {
        use crate::buffer::coding::looks_binary;
        assert!(looks_binary(b"\x7FELF\x02\x01\x01\x00"));
        assert!(!looks_binary(b"perch\xE9\n"));
        assert!(!looks_binary("perché\n".as_bytes()));
        // Only the first few thousand bytes are looked at.
        let mut late = vec![b'a'; 9000];
        late.push(0);
        assert!(!looks_binary(&late));
    }

    #[test]
    fn a_coding_is_found_by_its_name() {
        for coding in Coding::ALL {
            assert_eq!(Coding::named(coding.name()), Some(coding));
        }
        assert_eq!(Coding::named("UTF-8"), Some(Coding::Utf8));
        assert_eq!(Coding::named("latin-2"), None);
    }

    // ----------------------------------------------------------------
    // Through the editor
    // ----------------------------------------------------------------

    /// A directory of this test's own, removed when it goes.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rsedit-coding-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        /// Writes BYTES to a file called NAME, and returns its path.
        fn file(&self, name: &str, bytes: &[u8]) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).expect("writing a file");
            path.display().to_string()
        }

        /// Writes BYTES over NAME with a time in the future, so that the
        /// watcher sees a change however fast the test runs.
        fn rewrite(&self, name: &str, bytes: &[u8]) {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).expect("rewriting");
            let later = SystemTime::now() + Duration::from_secs(60);
            let _ = std::fs::File::open(&path).map(|file| file.set_modified(later));
        }

        fn read(&self, name: &str) -> Vec<u8> {
            std::fs::read(self.0.join(name)).expect("reading a file")
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// Opens PATH, and returns the buffer's name.
    fn open(path: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match run(&format!(r#"(find-file "{path}")"#), env, ctx) {
            LispExp::String(name) => name.to_string(),
            other => panic!("find-file answered {other:?}"),
        }
    }

    fn text(ctx: &Ctx, name: &str) -> String {
        ctx.with_buffer(name, |buf| buf.text.to_string())
            .expect("the buffer")
    }

    fn coding(ctx: &Ctx, name: &str) -> Coding {
        ctx.with_buffer(name, |buf| buf.coding).expect("the buffer")
    }

    fn modified(ctx: &Ctx, name: &str) -> bool {
        ctx.with_buffer(name, |buf| buf.is_modified)
            .expect("the buffer")
    }

    /// Appends TEXT at the end of the current buffer.
    fn append(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(&format!(r#"(end-of-buffer) (insert "{text}")"#), env, ctx);
    }

    #[test]
    fn a_file_not_in_utf8_opens_and_says_so() {
        let sandbox = Sandbox::new("opens");
        let path = sandbox.file("old.txt", b"perch\xE9\n");
        let (ctx, env) = editor();

        let name = open(&path, &env, &ctx);

        assert_eq!(text(&ctx, &name), "perché\n");
        assert_eq!(coding(&ctx, &name), Coding::Windows1252);
        assert!(!modified(&ctx, &name));
        assert_eq!(ctx.get_echo_message(), "Not UTF-8: read as windows-1252");
    }

    #[test]
    fn a_utf8_file_says_nothing_of_its_coding() {
        let sandbox = Sandbox::new("utf8");
        let path = sandbox.file("new.txt", "perché\n".as_bytes());
        let (ctx, env) = editor();
        ctx.set_echo_message("");

        let name = open(&path, &env, &ctx);

        assert_eq!(coding(&ctx, &name), Coding::Utf8);
        assert_eq!(ctx.get_echo_message(), "");
    }

    #[test]
    fn a_file_saved_unchanged_comes_back_byte_for_byte() {
        let sandbox = Sandbox::new("unchanged");
        let path = sandbox.file("binary", &every_byte());
        let (ctx, env) = editor();
        open(&path, &env, &ctx);

        run("(save-buffer)", &env, &ctx);

        assert_eq!(sandbox.read("binary"), every_byte());
    }

    #[test]
    fn what_is_typed_is_saved_in_the_files_coding() {
        let sandbox = Sandbox::new("typed");
        let path = sandbox.file("old.txt", b"perch\xE9\n");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);

        append("però €", &env, &ctx);
        run("(save-buffer)", &env, &ctx);

        assert_eq!(sandbox.read("old.txt"), b"perch\xE9\nper\xF2 \x80".to_vec());
    }

    #[test]
    fn a_character_the_coding_lacks_is_refused_and_nothing_is_written() {
        let sandbox = Sandbox::new("refused");
        let path = sandbox.file("old.txt", b"perch\xE9\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        append("\u{101}", &env, &ctx);
        run("(save-buffer)", &env, &ctx);

        assert_eq!(sandbox.read("old.txt"), b"perch\xE9\n".to_vec());
        assert!(modified(&ctx, &name), "the buffer still has work to save");
        let said = ctx.get_echo_message();
        assert!(
            said.starts_with(&format!("Cannot save {path} in windows-1252")),
            "{said:?}"
        );
    }

    #[test]
    fn set_buffer_file_coding_writes_the_next_save_in_another() {
        let sandbox = Sandbox::new("converted");
        let path = sandbox.file("old.txt", b"perch\xE9\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        append("\u{101}", &env, &ctx);

        assert_eq!(
            run(r#"(set-buffer-file-coding "utf-8")"#, &env, &ctx),
            LispExp::string("utf-8".into())
        );
        run("(save-buffer)", &env, &ctx);

        assert_eq!(
            sandbox.read("old.txt"),
            "perché\n\u{101}".as_bytes().to_vec()
        );
        assert_eq!(coding(&ctx, &name), Coding::Utf8);
        assert!(!modified(&ctx, &name));
    }

    #[test]
    fn setting_a_coding_marks_the_buffer_modified() {
        let sandbox = Sandbox::new("marked");
        let path = sandbox.file("old.txt", b"perch\xE9\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        run(r#"(set-buffer-file-coding 'utf-8)"#, &env, &ctx);

        assert!(modified(&ctx, &name), "there is a save to make");
    }

    #[test]
    fn an_unknown_coding_is_refused() {
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(set-buffer-file-coding "latin-2")"#, &env, &ctx),
            LispExp::nil()
        );
        assert_eq!(coding(&ctx, "*scratch*"), Coding::Utf8);
        assert!(
            ctx.get_echo_message()
                .starts_with("No coding called latin-2")
        );
    }

    #[test]
    fn buffer_file_coding_says_which() {
        let sandbox = Sandbox::new("says");
        let path = sandbox.file("old.txt", b"perch\xE9\n");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);

        assert_eq!(
            run("(buffer-file-coding)", &env, &ctx),
            LispExp::string("windows-1252".into())
        );
        assert_eq!(ctx.get_echo_message(), "Coding: windows-1252");
    }

    #[test]
    fn a_signature_survives_opening_and_saving() {
        let sandbox = Sandbox::new("signature");
        let path = sandbox.file("bom.txt", b"\xEF\xBB\xBFciao\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        assert_eq!(text(&ctx, &name), "ciao\n", "no invisible character");
        run("(save-buffer)", &env, &ctx);

        assert_eq!(sandbox.read("bom.txt"), b"\xEF\xBB\xBFciao\n".to_vec());
    }

    #[test]
    fn write_file_writes_in_the_buffers_coding() {
        let sandbox = Sandbox::new("write-file");
        let path = sandbox.file("old.txt", b"perch\xE9\n");
        let copy = sandbox.0.join("copy.txt").display().to_string();
        let (ctx, env) = editor();
        open(&path, &env, &ctx);

        run(&format!(r#"(write-file "{copy}")"#), &env, &ctx);

        assert_eq!(sandbox.read("copy.txt"), b"perch\xE9\n".to_vec());
    }

    #[test]
    fn revert_buffer_reads_a_file_that_is_no_longer_utf8() {
        let sandbox = Sandbox::new("revert");
        let path = sandbox.file("file.txt", "perché\n".as_bytes());
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        sandbox.rewrite("file.txt", b"cos\xEC\n");

        run("(revert-buffer)", &env, &ctx);

        assert_eq!(text(&ctx, &name), "così\n");
        assert_eq!(coding(&ctx, &name), Coding::Windows1252);
    }

    #[test]
    fn a_file_rewritten_on_disk_is_reloaded_in_its_coding() {
        let sandbox = Sandbox::new("reload");
        let path = sandbox.file("file.txt", b"perch\xE9\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        sandbox.rewrite("file.txt", "così\n".as_bytes());

        ctx.watch_files_turn(0);

        assert_eq!(text(&ctx, &name), "così\n");
        assert_eq!(coding(&ctx, &name), Coding::Utf8);
    }

    #[test]
    fn read_file_to_string_reads_a_file_that_is_not_utf8() {
        let sandbox = Sandbox::new("to-string");
        let path = sandbox.file("old.txt", b"perch\xE9");
        let (ctx, env) = editor();

        assert_eq!(
            run(&format!(r#"(read-file-to-string "{path}")"#), &env, &ctx),
            LispExp::string("perché".into())
        );
    }
}
