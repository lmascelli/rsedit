use super::*;

pub const FIND_FILE_DOC: &str = "(find-file PATH): Open the file at PATH into a new buffer named \
         after PATH's file name (or PATH itself if it has none), make it the \
         current buffer, and return its buffer name. Returns nil (logging a \
         diagnostic) if the file can't be read.\n\n\
         Example:\n\
         (find-file \"/home/me/notes.txt\") => \"notes.txt\"";

primitive!(find_file, args, _env, ctx, {
    if let Some(ELispExp::String(path_str)) = args.first() {
        let path_str = path_str.to_string();
        let path = std::path::Path::new(&path_str);
        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let buf_name = if file_name.is_empty() {
            path_str.to_string()
        } else {
            file_name
        };
        match ctx.new_buffer(&buf_name, Some(&path_str), None) {
            Some(buf_name) => Ok(ELispExp::string(buf_name)),
            None => Ok(ELispExp::nil()),
        }
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        })
    }
});

pub const SAVE_BUFFER_DOC: &str = "(save-buffer): Write the current buffer's contents to the file it \
         was visiting. Returns nil in every case (logging a diagnostic \
         either way); if the buffer has no associated file, or the write \
         fails, nothing is written.\n\n\
         Example:\n\
         (define-key nil \"C-x C-s\" 'save-buffer)";

primitive!(save_buffer, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("Failed to acquire write lock on buffer");
    let path = if let Some(path) = &buf.file_path {
        path.to_string()
    } else {
        ctx.log_diagnostic("No file associated with this buffer");
        return Ok(ELispExp::nil());
    };
    let content = buf.text.to_string();
    match std::fs::write(&path, content) {
        Ok(_) => {
            buf.is_modified = false;
            ctx.log_diagnostic(&format!("Wrote {}", path));
            Ok(ELispExp::nil())
        }
        Err(e) => {
            ctx.log_diagnostic(&format!("Failed to save: {}", e));
            Ok(ELispExp::nil())
        }
    }
});
