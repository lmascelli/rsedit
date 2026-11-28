//! The coding a file is read and written in: UTF-8, UTF-8 behind its
//! signature, and Windows-1252 for every file that is not UTF-8. The table on
//! its own first, then a file through the editor: opened, saved unchanged,
//! edited, refused a character its coding lacks, converted, reverted and
//! reloaded from the disk.
//!
//! Design: `doc/architettura-editor.typ, <codifiche>`.
#[cfg(test)]
mod tests {
    use crate::buffer::coding::{Coding, Eol, decode, decode_as, encode, first_invalid_byte};
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
            ("perché".to_string(), Coding::Utf8, Eol::Lf)
        );
        assert_eq!(
            decode(b"plain"),
            ("plain".to_string(), Coding::Utf8, Eol::Lf)
        );
        assert_eq!(decode(b""), (String::new(), Coding::Utf8, Eol::Lf));
    }

    #[test]
    fn latin_1_is_read_as_windows_1252() {
        assert_eq!(
            decode(b"perch\xE9 cos\xEC"),
            ("perché così".to_string(), Coding::Windows1252, Eol::Lf)
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
        let (text, coding, eol) = decode(&every_byte());
        assert_eq!(coding, Coding::Windows1252);
        let distinct: std::collections::HashSet<char> = text.chars().collect();
        assert_eq!(distinct.len(), 256);
        assert_eq!(encode(&text, coding, eol), Ok(every_byte()));
    }

    #[test]
    fn a_signature_is_taken_off_and_put_back() {
        let bytes = b"\xEF\xBB\xBFciao".to_vec();
        let (text, coding, eol) = decode(&bytes);
        assert_eq!((text.as_str(), coding), ("ciao", Coding::Utf8WithSignature));
        assert_eq!(encode(&text, coding, eol), Ok(bytes));
    }

    #[test]
    fn a_character_windows_1252_lacks_is_refused() {
        assert_eq!(
            encode("a\u{101}b", Coding::Windows1252, Eol::Lf),
            Err('\u{101}')
        );
        assert_eq!(encode("\u{80}", Coding::Windows1252, Eol::Lf), Err('\u{80}'));
        assert_eq!(
            encode("è€", Coding::Windows1252, Eol::Lf),
            Ok(b"\xE8\x80".to_vec())
        );
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
    // Line ends
    // ----------------------------------------------------------------

    /// Reads BYTES and writes them back as they were read.
    fn round_trip(bytes: &[u8]) -> Vec<u8> {
        let (text, coding, eol) = decode(bytes);
        encode(&text, coding, eol).expect("every character has its byte")
    }

    #[test]
    fn crlf_is_read_as_lf_and_written_back_as_crlf() {
        assert_eq!(
            decode(b"a\r\nb\r\n"),
            ("a\nb\n".to_string(), Coding::Utf8, Eol::Crlf)
        );
        assert_eq!(round_trip(b"a\r\nb\r\n"), b"a\r\nb\r\n".to_vec());
        assert_eq!(
            encode("aX\nb\n", Coding::Utf8, Eol::Crlf),
            Ok(b"aX\r\nb\r\n".to_vec())
        );
    }

    #[test]
    fn cr_alone_is_read_as_lf_and_written_back_as_cr() {
        assert_eq!(
            decode(b"a\rb\r"),
            ("a\nb\n".to_string(), Coding::Utf8, Eol::Cr)
        );
        assert_eq!(round_trip(b"a\rb\r"), b"a\rb\r".to_vec());
    }

    #[test]
    fn crlf_is_found_in_a_file_that_is_not_utf8() {
        assert_eq!(
            decode(b"perch\xE9\r\ncos\xEC\r\n"),
            ("perché\ncosì\n".to_string(), Coding::Windows1252, Eol::Crlf)
        );
        assert_eq!(
            round_trip(b"perch\xE9\r\ncos\xEC\r\n"),
            b"perch\xE9\r\ncos\xEC\r\n".to_vec()
        );
    }

    #[test]
    fn a_lone_cr_in_a_crlf_file_is_kept_where_it_was() {
        let bytes = b"a\r\nb\rc\r\nd\r\n";
        assert_eq!(
            decode(bytes),
            ("a\nb\rc\nd\n".to_string(), Coding::Utf8, Eol::Crlf)
        );
        assert_eq!(round_trip(bytes), bytes.to_vec());
    }

    #[test]
    fn a_mixed_file_takes_the_line_end_most_of_its_lines_have() {
        // Mostly CRLF: the one LF is written back as the others are.
        let (text, coding, eol) = decode(b"a\r\nb\nc\r\n");
        assert_eq!((text.as_str(), eol), ("a\nb\nc\n", Eol::Crlf));
        assert_eq!(encode(&text, coding, eol), Ok(b"a\r\nb\r\nc\r\n".to_vec()));
        // Mostly LF: the CR stays in the text, and comes back.
        let (text, _, eol) = decode(b"a\nb\r\nc\n");
        assert_eq!((text.as_str(), eol), ("a\nb\r\nc\n", Eol::Lf));
        assert_eq!(round_trip(b"a\nb\r\nc\n"), b"a\nb\r\nc\n".to_vec());
        // A tie changes nothing.
        assert_eq!(decode(b"a\r\nb\n").2, Eol::Lf);
    }

    #[test]
    fn only_the_first_lines_are_counted() {
        let mut bytes = b"x\r\n".repeat(100);
        bytes.extend(b"y\n".repeat(200));
        assert_eq!(decode(&bytes).2, Eol::Crlf);
    }

    #[test]
    fn a_file_with_no_line_end_or_a_binary_one_is_lf() {
        assert_eq!(Eol::detect("plain"), None);
        assert_eq!(decode(b"plain").2, Eol::Lf);
        let binary = b"\x00\r\n\x01\r\n\x02\n";
        assert_eq!(decode(binary).2, Eol::Lf);
        assert_eq!(round_trip(binary), binary.to_vec());
    }

    #[test]
    fn a_line_end_is_found_by_its_name_or_its_characters() {
        for eol in Eol::ALL {
            assert_eq!(Eol::named(eol.name()), Some(eol));
        }
        assert_eq!(Eol::named("DOS"), Some(Eol::Crlf));
        assert_eq!(Eol::named("crlf"), Some(Eol::Crlf));
        assert_eq!(Eol::named("lf"), Some(Eol::Lf));
        assert_eq!(Eol::named("cr"), Some(Eol::Cr));
        assert_eq!(Eol::named("ebcdic"), None);
    }

    // ----------------------------------------------------------------
    // A coding asked for
    // ----------------------------------------------------------------

    #[test]
    fn bytes_are_read_in_the_coding_asked_for_or_not_at_all() {
        assert_eq!(
            decode_as("perché\r\n".as_bytes(), Coding::Utf8),
            Some(("perché\n".to_string(), Eol::Crlf))
        );
        assert_eq!(
            decode_as("perché".as_bytes(), Coding::Windows1252),
            Some(("perchÃ©".to_string(), Eol::Lf))
        );
        assert_eq!(
            decode_as(b"\xEF\xBB\xBFciao", Coding::Utf8WithSignature),
            Some(("ciao".to_string(), Eol::Lf))
        );
        assert_eq!(
            decode_as(b"ciao", Coding::Utf8WithSignature),
            Some(("ciao".to_string(), Eol::Lf))
        );
        assert_eq!(decode_as(b"perch\xE9", Coding::Utf8), None);
        assert_eq!(decode_as(b"\xEF\xBB\xBF\xE9", Coding::Utf8WithSignature), None);
    }

    #[test]
    fn the_first_byte_that_is_not_utf8_is_found_by_line_and_column() {
        let (text, ..) = decode(b"ok\r\nperch\xE9 cos\xEC\r\n");
        assert_eq!(first_invalid_byte(&text), Some((2, 5)));
        // A lone CR is a byte of the line it is on.
        let (text, ..) = decode(b"a\nb\r\xE9\n");
        assert_eq!(first_invalid_byte(&text), Some((2, 2)));
        // A character cut short at the end of a line is not UTF-8 either.
        let (text, ..) = decode(b"\xC3\nb");
        assert_eq!(first_invalid_byte(&text), Some((1, 0)));
        assert_eq!(first_invalid_byte("plain\ntext"), None);
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

    fn eol(ctx: &Ctx, name: &str) -> Eol {
        ctx.with_buffer(name, |buf| buf.eol).expect("the buffer")
    }

    /// Returns the window showing NAME.
    fn view(ctx: &Ctx, env: &Arc<Env<Ctx>>, name: &str) -> crate::ui::RenderableWindowView {
        ctx.snapshot(env, 80, 24)
            .views
            .into_iter()
            .find(|view| view.buffer_name == name)
            .expect("the buffer is on screen")
    }

    #[test]
    fn a_crlf_file_is_edited_at_the_end_of_a_line_and_saved_as_crlf() {
        let sandbox = Sandbox::new("crlf-edit");
        let path = sandbox.file("dos.txt", b"a\r\nb\r\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        assert_eq!(text(&ctx, &name), "a\nb\n");
        assert_eq!(eol(&ctx, &name), Eol::Crlf);

        // What C-e runs: it stops before the line end, not between its CR and
        // its LF.
        run("(beginning-of-buffer) (end-of-line)", &env, &ctx);
        assert_eq!(
            ctx.with_buffer(&name, |buf| crate::buffer::BufferTrait::cursor_pos(&buf.text)),
            Some((0, 1))
        );
        run(r#"(insert "X")"#, &env, &ctx);
        let shown = view(&ctx, &env, &name);
        assert!(
            shown
                .lines
                .iter()
                .all(|line| !line.contains(['\r', '\u{240D}']) && !line.contains("^M")),
            "no CR on screen: {:?}",
            shown.lines
        );
        run("(save-buffer)", &env, &ctx);

        assert_eq!(sandbox.read("dos.txt"), b"aX\r\nb\r\n".to_vec());
        assert!(!modified(&ctx, &name));
    }

    #[test]
    fn a_lone_cr_in_a_crlf_file_survives_opening_and_saving() {
        let sandbox = Sandbox::new("crlf-lone-cr");
        let path = sandbox.file("dos.txt", b"a\r\nb\rc\r\nd\r\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        assert_eq!(text(&ctx, &name), "a\nb\rc\nd\n");

        append("e", &env, &ctx);
        run("(save-buffer)", &env, &ctx);

        assert_eq!(sandbox.read("dos.txt"), b"a\r\nb\rc\r\nd\r\ne".to_vec());
    }

    #[test]
    fn the_mode_line_says_when_lines_do_not_end_in_lf() {
        let sandbox = Sandbox::new("crlf-mode-line");
        let dos = sandbox.file("dos.txt", b"a\r\n");
        let mac = sandbox.file("mac.txt", b"a\r");
        let unix = sandbox.file("unix.txt", b"a\n");
        let (ctx, env) = editor();
        run(r#"(setq mode-line-format "%*")"#, &env, &ctx);

        for (path, said) in [(dos, "--(DOS)"), (mac, "--(Mac)"), (unix, "--")] {
            let name = open(&path, &env, &ctx);
            assert_eq!(view(&ctx, &env, &name).mode_line.as_deref(), Some(said));
        }
    }

    #[test]
    fn set_buffer_file_eol_writes_the_next_save_with_another() {
        let sandbox = Sandbox::new("crlf-set");
        let path = sandbox.file("unix.txt", b"a\nb\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        assert_eq!(
            run(r#"(set-buffer-file-eol "dos")"#, &env, &ctx),
            LispExp::string("dos".into())
        );
        assert!(modified(&ctx, &name), "there is a save to make");
        run("(save-buffer)", &env, &ctx);
        assert_eq!(sandbox.read("unix.txt"), b"a\r\nb\r\n".to_vec());

        run("(set-buffer-file-eol 'unix) (save-buffer)", &env, &ctx);
        assert_eq!(sandbox.read("unix.txt"), b"a\nb\n".to_vec());

        assert_eq!(
            run(r#"(set-buffer-file-eol "ebcdic")"#, &env, &ctx),
            LispExp::nil()
        );
        assert_eq!(eol(&ctx, &name), Eol::Lf);
        assert!(ctx.get_echo_message().starts_with("No line end called ebcdic"));
    }

    #[test]
    fn write_file_and_revert_keep_the_line_ends() {
        let sandbox = Sandbox::new("crlf-write-revert");
        let path = sandbox.file("dos.txt", b"a\r\n");
        let copy = sandbox.0.join("copy.txt").display().to_string();
        let (ctx, env) = editor();
        open(&path, &env, &ctx);

        run(&format!(r#"(write-file "{copy}")"#), &env, &ctx);
        assert_eq!(sandbox.read("copy.txt"), b"a\r\n".to_vec());

        sandbox.rewrite("copy.txt", b"b\rc\r");
        run("(revert-buffer)", &env, &ctx);
        let name = ctx.get_current_buffer_name().to_string();
        assert_eq!(text(&ctx, &name), "b\nc\n");
        assert_eq!(eol(&ctx, &name), Eol::Cr);
    }

    #[test]
    fn a_crlf_file_rewritten_on_disk_is_reloaded_without_its_crs() {
        let sandbox = Sandbox::new("crlf-reload");
        let path = sandbox.file("dos.txt", b"a\r\n");
        let (ctx, env) = editor();
        crate::tests::shipped::load_auto_revert(&env, &ctx);
        let name = open(&path, &env, &ctx);
        sandbox.rewrite("dos.txt", b"a\r\nb\r\n");

        ctx.watch_files_turn(0);

        assert_eq!(text(&ctx, &name), "a\nb\n");
        assert_eq!(eol(&ctx, &name), Eol::Crlf);
        assert!(!modified(&ctx, &name));
        append("c", &env, &ctx);
        run("(save-buffer)", &env, &ctx);
        assert_eq!(sandbox.read("dos.txt"), b"a\r\nb\r\nc".to_vec());

        // A file that changes its line ends on disk changes the buffer's.
        sandbox.rewrite("dos.txt", b"x\ny\n");
        ctx.watch_files_turn(0);
        assert_eq!(text(&ctx, &name), "x\ny\n");
        assert_eq!(eol(&ctx, &name), Eol::Lf);
    }

    #[test]
    fn the_asynchronous_commands_keep_the_line_ends() {
        use crate::tests::file_io_tests::settle;
        let sandbox = Sandbox::new("crlf-async");
        let path = sandbox.file("dos.txt", b"a\r\nb\r\n");
        let (ctx, env) = editor();

        run(&format!(r#"(find-file-async "{path}")"#), &env, &ctx);
        settle(&ctx, &env);
        let name = ctx.get_current_buffer_name().to_string();
        assert_eq!(text(&ctx, &name), "a\nb\n");
        assert_eq!(eol(&ctx, &name), Eol::Crlf);

        run(
            r#"(beginning-of-buffer) (end-of-line) (insert "X") (save-buffer-async)"#,
            &env,
            &ctx,
        );
        settle(&ctx, &env);
        assert_eq!(sandbox.read("dos.txt"), b"aX\r\nb\r\n".to_vec());

        sandbox.rewrite("dos.txt", b"c\r\nd\r\n");
        run("(revert-buffer-async)", &env, &ctx);
        settle(&ctx, &env);
        assert_eq!(text(&ctx, &name), "c\nd\n");
        assert_eq!(eol(&ctx, &name), Eol::Crlf);
    }

    #[test]
    fn the_auto_save_copy_has_the_files_line_ends_and_recovers_without_them() {
        let sandbox = Sandbox::new("crlf-auto-save");
        let path = sandbox.file("dos.txt", b"a\r\n");
        let (ctx, env) = editor();
        env.set_variable("auto-save-directory".into(), LispExp::nil());
        let name = open(&path, &env, &ctx);
        append("b", &env, &ctx);

        let pending = ctx.auto_save_for(&name, None).expect("a copy to write");
        ctx.write_auto_save(pending);
        let copy = crate::jobs::autosave::auto_save_path(&path, None);
        assert_eq!(std::fs::read(&copy).expect("the copy"), b"a\r\nb".to_vec());

        run("(recover-file--adopt)", &env, &ctx);
        assert_eq!(text(&ctx, &name), "a\nb");
        assert_eq!(eol(&ctx, &name), Eol::Crlf);
    }

    /// Types TEXT into the question asked, and Return.
    fn answer(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for c in text.chars() {
            run(&format!(r#"(self-insert "{c}")"#), env, ctx);
        }
        run("(minibuffer-confirm)", env, ctx);
    }

    #[test]
    fn revert_buffer_with_coding_reads_the_file_again_in_another() {
        let sandbox = Sandbox::new("revert-with");
        let path = sandbox.file("file.txt", "perché\r\n".as_bytes());
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        assert_eq!(
            run(r#"(revert-buffer-with-coding "windows-1252")"#, &env, &ctx),
            LispExp::t()
        );
        assert_eq!(text(&ctx, &name), "perchÃ©\n");
        assert_eq!(coding(&ctx, &name), Coding::Windows1252);
        assert_eq!(eol(&ctx, &name), Eol::Crlf);
        assert_eq!(
            ctx.get_echo_message(),
            format!("Reverted to {path} in windows-1252")
        );
        // Saved unchanged in the coding asked for, it comes back as it was.
        run("(save-buffer)", &env, &ctx);
        assert_eq!(sandbox.read("file.txt"), "perché\r\n".as_bytes().to_vec());

        run("(revert-buffer-with-coding 'utf-8)", &env, &ctx);
        assert_eq!(text(&ctx, &name), "perché\n");
        assert_eq!(coding(&ctx, &name), Coding::Utf8);
    }

    #[test]
    fn a_file_that_is_not_utf8_is_not_read_in_utf8_and_says_where() {
        let sandbox = Sandbox::new("revert-with-refused");
        let path = sandbox.file("old.txt", b"ok\nperch\xE9\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        assert_eq!(
            run(r#"(revert-buffer-with-coding "utf-8")"#, &env, &ctx),
            LispExp::nil()
        );
        assert_eq!(
            ctx.get_echo_message(),
            format!("Cannot read {path} in utf-8: not UTF-8 at line 2, column 5")
        );
        assert_eq!(text(&ctx, &name), "ok\nperché\n", "left as it was");
        assert_eq!(coding(&ctx, &name), Coding::Windows1252);

        assert_eq!(
            run(r#"(revert-buffer-with-coding "latin-2")"#, &env, &ctx),
            LispExp::nil()
        );
        assert!(ctx.get_echo_message().starts_with("No coding called latin-2"));
    }

    #[test]
    fn revert_buffer_with_coding_asks_before_losing_changes() {
        let sandbox = Sandbox::new("revert-with-asked");
        let path = sandbox.file("file.txt", "perché\n".as_bytes());
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        append("mine", &env, &ctx);

        run(r#"(revert-buffer-with-coding "windows-1252")"#, &env, &ctx);
        assert_eq!(text(&ctx, &name), "perché\nmine", "nothing yet");
        answer("yes", &env, &ctx);

        assert_eq!(text(&ctx, &name), "perchÃ©\n");
        assert_eq!(coding(&ctx, &name), Coding::Windows1252);
        assert!(!modified(&ctx, &name));
    }

    #[test]
    fn a_revert_and_an_asynchronous_open_say_where_the_file_is_not_utf8() {
        use crate::tests::file_io_tests::settle;
        let sandbox = Sandbox::new("not-utf8-where");
        let path = sandbox.file("old.txt", "perché\n".as_bytes());
        let (ctx, env) = editor();
        open(&path, &env, &ctx);
        sandbox.rewrite("old.txt", b"ok\r\ncos\xEC\r\n");

        run("(revert-buffer)", &env, &ctx);
        assert_eq!(
            ctx.get_echo_message(),
            format!("Reverted to {path}; not UTF-8 at line 2, column 3: read as windows-1252")
        );

        let other = sandbox.file("other.txt", b"\xE0 la\n");
        run(&format!(r#"(find-file-async "{other}")"#), &env, &ctx);
        settle(&ctx, &env);
        assert_eq!(
            ctx.get_echo_message(),
            format!("Opened {other}; not UTF-8 at line 1, column 0: read as windows-1252")
        );
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
        assert_eq!(
            ctx.get_echo_message(),
            "Not UTF-8 at line 1, column 5: read as windows-1252"
        );
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
        crate::tests::shipped::load_auto_revert(&env, &ctx);
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
