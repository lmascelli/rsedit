use super::*;

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
            got: format!("{:?}", args.first()),
        })
    }
});

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


