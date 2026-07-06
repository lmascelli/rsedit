use crate::buffer::{Buffer, BufferTrait};
use crate::input::{KeyCode, KeyEvent, default_keymaps};
use crate::lisp::{Env, EvalError, LispContext, LispExp, eval};
use crate::ui::{
    FloatingWindow, LayoutNode, Rect, RenderableWindowView, Window, extract_buffer_lines,
};
use std::{collections::HashMap, sync::Arc};
pub type ELispExp<B> = LispExp<EditorState<B>>;

#[derive(Clone, Debug)]
pub struct MajorMode<B: BufferTrait> {
    pub name: String,
    pub keymap: HashMap<KeyEvent, ELispExp<B>>,
    pub syntax_highlighing: (), // TODO! make it a SyntaxRules
    pub hook_functions: Vec<ELispExp<B>>,
}

/// This is the container for all the editor informations.
/// The whole editor memory should live in an instance of this
/// struct. It is generic behiond the implementation of the
/// buffer.
/// It also provides instruction for an UI provider of what to
/// render and where.
pub struct EditorState<B: BufferTrait> {
    pub running: bool,

    pub buffers: HashMap<String, Buffer<B>>,
    pub current_buffer_name: String,
    pub echo_message: String,

    /// A keymap is an association between a KeyEvent and the name of a 
    /// function that have to be executed (i.e. self-insert)
    pub keymaps: HashMap<KeyEvent, String>,
    pub mode_registry: HashMap::<String, MajorMode<B>>,
    /// This is the root of the window tree that the UI should visualize
    pub tiled_root: LayoutNode,
    /// This is a list of floating window that will be renderered above the
    /// others
    pub floating_windows: Vec<FloatingWindow>,
    pub focused_window_id: usize,
    /// A value only used to fastly create a new window id
    pub next_window_id: usize,

    /// The fuel of the lisp machine, if somehow it will start to use too much
    /// cpu power, it will run out of fuel
    fuel: u32,
    /// Here the lisp VM will output its logs
    logs: Vec<String>,
}

impl<B: BufferTrait> LispContext for EditorState<B> {
    fn consume_fuel(&mut self, amount: u32) -> Result<(), EvalError> {
        if self.fuel > amount {
            self.fuel -= amount;
            Ok(())
        } else {
            self.fuel = 0;
            Err(EvalError::OutOfFuel)
        }
    }

    fn log_diagnostic(&mut self, msg: &str) {
        self.logs.push(msg.into());
    }
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
    /// Create a new EditorState environment. Install the default keymaps, 
    /// provides a default *scratch* buffer in a base window.
    fn new() -> Self {
        let mut buffers = HashMap::new();
        let scratch_name = "*scratch*".to_string();
        buffers.insert(scratch_name.clone(), Buffer::new(&scratch_name));

        let keymaps = default_keymaps();

        Self {
            buffers,
            current_buffer_name: scratch_name,
            tiled_root: LayoutNode::Leaf(Window {
                id: 0,
                buffer_name: String::from("*scratch*"),
                scroll_x: 0,
                scroll_y: 0,
            }),
            floating_windows: Vec::new(),
            echo_message: "Welcome to rsedit".to_string(),
            keymaps,
            running: true,
            focused_window_id: 0,
            next_window_id: 1,
            mode_registry: HashMap::new(),
            fuel: 10000,
            logs: vec![],
        }
    }

    /// Open a new empty buffer or load a file into a new buffer if a path is
    /// provided.
    fn new_buffer(&mut self, name: &str, path: Option<&str>) -> Option<String> {
        if let Some(file_path) = path {
            match std::fs::read_to_string(file_path) {
                Ok(content) => {
                    let mut new_buf = Buffer::from_text(name, &content);
                    new_buf.file_path = Some(file_path.to_string());

                    self.buffers.insert(name.to_string(), new_buf);

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

    /// Returns a mutable reference to the current buffer
    fn current_buffer_mut(&mut self) -> &mut Buffer<B> {
        self.buffers
            .get_mut(&self.current_buffer_name)
            .expect("Corruption in the hashmap of buffers")
    }

    /// Returns an immutable reference to the current buffer
    fn current_buffer(&self) -> &Buffer<B> {
        self.buffers
            .get(&self.current_buffer_name)
            .expect("Corruption in the hashmap of buffers")
    }

    /// Handle a key event. An UI provider is responsible to call this function
    /// every time it want to make the editor react to an user input.
    pub fn handle_key_event(&mut self, event: KeyEvent, env: &Arc<Env<EditorState<B>>>) {
        if let Some(symbol_name) = self.keymaps.get(&event) {
            let mut ast = vec![ELispExp::symbol(symbol_name.clone())];
            if let KeyCode::Char(c) = event.code {
                ast.push(ELispExp::string(c.to_string()));
            }
            let ast = ELispExp::list(ast);
            if let Err(e) = eval(&ast, env.clone(), self) {
                self.echo_message = format!("Eval Error: {:?} {:?}", ast, e);
            } else {
                self.echo_message.clear();
            }
        } else {
            self.echo_message = format!("Keymap not bound {:?}", event);
        }
    }

    /// Ask the editor for a list of window to be rendered. Those are composed of a rect that tells
    /// where the window is placed and its size, the name of the buffer it represents, if it's
    /// focused, the relative cursor position in it, if it has a border and of course the line
    /// that it contains and that have to be drawn.
    pub fn compose_layout(
        &mut self,
        screen_width: usize,
        screen_height: usize,
    ) -> Vec<RenderableWindowView> {
        let mut views = Vec::new();
        let tiled_space = Rect {
            x: 0,
            y: 0,
            width: screen_width,
            height: screen_height,
        };
        self.tiled_root.compute_tiled_views(
            tiled_space,
            self.focused_window_id,
            &self.buffers,
            &mut views,
        );

        for float in &self.floating_windows {
            let lines = extract_buffer_lines(&float.window, &float.rect, &self.buffers);
            let is_focused = float.window.id == self.focused_window_id;

            let cursor_rel_pos = if is_focused {
                if let Some(buf) = self.buffers.get(&float.window.buffer_name) {
                    let (c_line, c_col) = buf.text.cursor_pos();
                    Some((
                        c_col.saturating_sub(float.window.scroll_x),
                        c_line.saturating_sub(float.window.scroll_y),
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            views.push(RenderableWindowView {
                rect: float.rect.clone(),
                buffer_name: float.window.buffer_name.clone(),
                is_focused,
                cursor_rel_pos,
                lines,
                has_border: float.has_border,
            });
        }

        views
    }
}

/// Create a global EditorState environment and a Lisp environment associated to it.
/// It installs in the lisp environment all the primitive functions to use the editor.
/// It is mandatory that the lisp environment does not outlive the EditorState struct.
pub fn create_global_env<B: BufferTrait>() -> (EditorState<B>, Arc<Env<EditorState<B>>>) {
    let editor_state = EditorState::new();
    let env = Env::new_root();

    macro_rules! insert_fn {
        ($name:literal, $func:ident) => {
            env.set_function($name.into(), LispExp::Primitive(primitives::$func));
        };
    }
    insert_fn!("quit", quit);
    insert_fn!("load-file", load_file);
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
            || (args.len() == 1 && (args[0] == ELispExp::list(vec![]))
                || args[0] == ELispExp::symbol("nil".into()))
    }

    macro_rules! nil {
        () => {
            ELispExp::symbol("nil".into())
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

    primitive!(load_file, args, ctx, {
        if let Some(ELispExp::String(path_str)) = args.first() {
            match std::fs::read_to_string(path_str.to_string()) {
                Ok(content) => {
                    let wrapped_content = format!("(progn {})", content);
                    let mut parser = crate::lisp::Parser::new(&wrapped_content);
                    match parser.next() {
                        Ok(ast) => Ok(ast),
                        Err(e) => {
                            ctx.echo_message = format!("Parse Error in {}: {:?}", path_str, e);
                            Ok(nil!())
                        }
                    }
                }
                Err(e) => {
                    ctx.echo_message = format!("Could not load {}: {}", path_str, e);
                    Ok(nil!())
                }
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: format!("{:?}", args.first()),
            })
        }
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
            Ok(LispExp::symbol("nil".into()))
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
        Ok(LispExp::symbol("nil".into()))
    });

    primitive!(delete_backward_char, _args, ctx, {
        let buf = ctx.current_buffer_mut();
        buf.text.delete();
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
            match ctx.new_buffer(&buf_name, Some(&path_str)) {
                Some(buf_name) => Ok(ELispExp::string(buf_name)),
                None => Ok(nil!()),
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
