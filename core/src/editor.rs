use crate::buffer::BufferTrait;
use crate::input::{KeyCode, KeyEvent, default_keymaps};
use crate::lisp::{Env, LispExp, eval};
use std::collections::HashMap;
pub type ELispExp<B> = LispExp<EditorState<B>>;

pub struct Buffer<B: BufferTrait> {
    pub name: String,
    pub text: B,
    pub file_path: Option<String>,
    pub is_modified: bool,

    // viewport
    pub scroll_x: usize,
    pub scroll_y: usize,
}

impl<B: BufferTrait> Buffer<B> {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            text: B::default(),
            file_path: None,
            is_modified: false,
            scroll_x: 0,
            scroll_y: 0,
        }
    }

    pub fn from_text(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            text: B::from_text(text),
            file_path: None,
            is_modified: false,
            scroll_x: 0,
            scroll_y: 0,
        }
    }
}

pub struct EditorState<B: BufferTrait> {
    pub buffers: HashMap<String, Buffer<B>>,
    pub current_buffer_name: String,
    pub echo_message: String,
    pub keymaps: HashMap<KeyEvent, String>,
    pub running: bool,
}

impl<B: BufferTrait> Clone for EditorState<B> {
    fn clone(&self) -> Self {
        unreachable!()
    }
}
impl<B: BufferTrait> std::fmt::Debug for EditorState<B> {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        todo!()
    }
}
impl<B: BufferTrait> std::cmp::PartialEq for EditorState<B> {
    fn eq(&self, _: &EditorState<B>) -> bool {
        unreachable!()
    }
}

impl<B: BufferTrait> EditorState<B> {
    pub fn new() -> Self {
        let mut buffers = HashMap::new();
        let scratch_name = "*scratch*".to_string();
        buffers.insert(scratch_name.clone(), Buffer::new(&scratch_name));

        let keymaps = default_keymaps();

        Self {
            buffers,
            current_buffer_name: scratch_name,
            echo_message: "Welcome to rsedit".to_string(),
            keymaps,
            running: true,
        }
    }

    pub fn new_buffer(&mut self, name: &str, path: Option<&str>) -> Option<String> {
        if let Some(file_path) = path {
            match std::fs::read_to_string(file_path) {
                Ok(content) => {
                    let mut new_buf = Buffer::from_text(name, &content);
                    new_buf.file_path = Some(file_path.to_string());

                    self.buffers
                        .insert(name.to_string(), new_buf);

                    self.current_buffer_name = name.to_string();

                    self.echo_message = format!("Loaded {}", file_path);
                    Some(name.to_string())
                }
                Err(e) => {
                    self.echo_message = format!("Error reading file: {}", e);
                    None
                }
            }
        } else {
            self.buffers.insert(name.to_string(), Buffer::new(name));
            Some(name.to_string())
        }
    }

    pub fn current_buffer_mut(&mut self) -> &mut Buffer<B> {
        self.buffers
            .get_mut(&self.current_buffer_name)
            .expect("Corruption in the hashmap of buffers")
    }

    pub fn current_buffer(&self) -> &Buffer<B> {
        self.buffers
            .get(&self.current_buffer_name)
            .expect("Corruption in the hashmap of buffers")
    }

    pub fn adjust_scroll(&mut self, viewport_width: usize, viewport_height: usize) {
        let text_area_height = if viewport_height > 1 {
            viewport_height - 1
        } else {
            1
        };
        let buf = self.current_buffer_mut();
        let (cursor_line, cursor_col) = buf.text.cursor_pos();

        if cursor_line < buf.scroll_y {
            buf.scroll_y = cursor_line; // Scroll up
        } else if cursor_line >= buf.scroll_y + text_area_height {
            buf.scroll_y = cursor_line - text_area_height + 1; // Scroll down
        }

        if cursor_col < buf.scroll_x {
            buf.scroll_x = cursor_col; // Scroll left
        } else if cursor_col >= buf.scroll_x + viewport_width {
            buf.scroll_x = cursor_col - viewport_width + 1; // Scroll right
        }
    }

    pub fn handle_key_event(&mut self, event: KeyEvent, env: &mut Env<EditorState<B>>) {
        if let Some(symbol_name) = self.keymaps.get(&event) {
            let mut ast = vec![ELispExp::Symbol(symbol_name.clone())];
            if let KeyCode::Char(c) = event.code {
                ast.push(ELispExp::String(c.to_string()));
            }
            let ast = ELispExp::List(ast);
            if let Err(e) = eval(&ast, env, self) {
                self.echo_message = format!("Eval Error: {:?} {:?}", ast, e);
            } else {
                self.echo_message.clear();
            }
        } else {
            self.echo_message = format!("Keymap not bound {:?}", event);
        }
    }
}

pub fn create_global_env<B: BufferTrait>() -> (EditorState<B>, Env<EditorState<B>>) {
    let editor_state = EditorState::new();
    let mut env = Env::new();

    macro_rules! insert_fn {
        ($name:literal, $func:ident) => {
            env.functions
                .insert($name.into(), LispExp::Primitive(primitives::$func));
        };
    }
    insert_fn!("quit", quit);
    insert_fn!("self-insert", self_insert);
    insert_fn!("insert-newline", insert_newline);
    insert_fn!("delete-backward-char", delete_backward_char);
    insert_fn!("backward-char", backward_char);
    insert_fn!("forward-char", forward_char);
    insert_fn!("previous-line", previous_line);
    insert_fn!("next-line", next_line);
    insert_fn!("find-file", find_file);
    insert_fn!("save-buffer", save_buffer);

    (editor_state, env)
}

// ---------------------------------------------------------------------------//
//                                                                            //
//                                  PRIMITIVES                                //
//                                                                            //
// ---------------------------------------------------------------------------//

mod primitives {
    use super::{BufferTrait, ELispExp, EditorState};
    use crate::lisp::{EvalError, LispExp};

    fn is_nil<B: BufferTrait>(args: &[ELispExp<B>]) -> bool {
        args.len() == 0
            || (args.len() == 1 && (args[0] == ELispExp::List(vec![]))
                || args[0] == ELispExp::Symbol("nil".into()))
    }

    macro_rules! nil {
        () => {
            ELispExp::Symbol("nil".into())
        };
    }

    macro_rules! primitive {
        ($func_name:ident, $args:ident, $ctx:ident, $body:block) => {
            pub fn $func_name<B: BufferTrait>(
                $args: &[ELispExp<B>],
                $ctx: &mut EditorState<B>,
            ) -> Result<ELispExp<B>, EvalError> {
                $body
            }
        };
    }

    //------------------------------------------------------------//
    //                                                            //
    //                          EDITOR                            //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(quit, _args, ctx, {
        ctx.running = false;
        Ok(nil!())
    });

    //------------------------------------------------------------//
    //                                                            //
    //                          EDIT                              //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(self_insert, args, ctx, {
        if let Some(LispExp::String(s)) = args.first() {
            if let Some(c) = s.chars().next() {
                let buf = ctx.current_buffer_mut();
                buf.text.insert(c);
                buf.is_modified = true;
            }
            Ok(LispExp::Symbol("nil".into()))
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: format!("{:?}", args.first()),
            })
        }
    });

    primitive!(insert_newline, _args, ctx, {
        let buf = ctx.current_buffer_mut();
        buf.text.insert('\n');
        buf.is_modified = true;
        Ok(LispExp::Symbol("nil".into()))
    });

    primitive!(delete_backward_char, _args, ctx, {
        let buf = ctx.current_buffer_mut();
        buf.text.delete_char();
        buf.is_modified = true;
        Ok(nil!())
    });

    //------------------------------------------------------------//
    //                                                            //
    //                     CURSOR MOVEMENT                        //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(forward_char, args, ctx, {
        let buf = ctx.current_buffer_mut();
        let step = if is_nil(args) {
            1
        } else {
            if let ELispExp::Number(n) = args[0] {
                n.floor() as usize
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: format!("{:?}", args[0]),
                });
            }
        };
        let (line, col) = buf.text.cursor_pos();
        buf.text.cursor_move(line, col + step);

        Ok(nil!())
    });

    primitive!(backward_char, args, ctx, {
        let buf = ctx.current_buffer_mut();
        let step = if is_nil(args) {
            1
        } else {
            if let ELispExp::Number(n) = args[0] {
                n.floor() as usize
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: format!("{:?}", args[0]),
                });
            }
        };
        let (line, col) = buf.text.cursor_pos();
        if col >= step {
            buf.text.cursor_move(line, col - step);
        } else {
            buf.text.cursor_move(line, 0);
        }
        Ok(nil!())
    });

    primitive!(previous_line, _args, ctx, {
        let buf = ctx.current_buffer_mut();
        let (line, col) = buf.text.cursor_pos();
        if line > 0 {
            buf.text.cursor_move(line - 1, col);
        } else {
            buf.text.cursor_move(0, 0);
        }
        Ok(nil!())
    });

    primitive!(next_line, _args, ctx, {
        let buf = ctx.current_buffer_mut();
        let (line, col) = buf.text.cursor_pos();
        buf.text.cursor_move(line + 1, col);
        Ok(nil!())
    });

    //------------------------------------------------------------//
    //                                                            //
    //                          I/O                               //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(find_file, args, ctx, {
        if let Some(ELispExp::String(path_str)) = args.first() {
            let path = std::path::Path::new(path_str);
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
            match ctx.new_buffer(&buf_name, Some(&path_str)) {
                Some(buf_name) => Ok(ELispExp::String(buf_name)),
                None => Ok(nil!())
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: format!("{:?}", args.first()),
            })
        }
    });

    primitive!(save_buffer, _args, ctx, {
        let buf = ctx.current_buffer_mut();
        if let Some(path) = &buf.file_path {
            let content = buf.text.to_string();
            match std::fs::write(path, content) {
                Ok(_) => {
                    buf.is_modified = false;
                    ctx.echo_message = format!("Wrote {}", path);
                    Ok(nil!())
                }
                Err(e) => {
                    ctx.echo_message = format!("Failed to save: {}", e);
                    Ok(nil!())
                }
            }
        } else {
            ctx.echo_message = "No file associated with this buffer".to_string();
            Ok(nil!())
        }
    });
}
