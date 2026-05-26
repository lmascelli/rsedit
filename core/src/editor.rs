use std::collections::HashMap;
use crate::input::{default_keymaps, KeyCode, KeyEvent};
use crate::buffer::GapBuffer;
use crate::lisp::{eval, Env, LispExp};
pub type ELispExp = LispExp<EditorState>;

pub struct Buffer {
    pub name: String,
    pub text: GapBuffer,
    pub file_path: Option<String>,
    pub is_modified: bool,
}

impl Buffer {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            text: GapBuffer::new(),
            file_path: None,
            is_modified: false,
        }
    }
}

pub struct EditorState {
    pub buffers: HashMap<String, Buffer>,
    pub current_buffer_name: String,
    pub echo_message: String,
    pub keymaps: HashMap<KeyEvent, String>,
    pub running: bool,
}

impl Clone for EditorState {
    fn clone(&self) -> Self {
        unreachable!()
    }
}

impl std::fmt::Debug for EditorState {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> { todo!() }
}

impl std::cmp::PartialEq for EditorState {
    fn eq(&self, _: &EditorState) -> bool { todo!() }
}

impl EditorState {
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

    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        self.buffers
            .get_mut(&self.current_buffer_name)
            .expect("Corruption in the hashmap of buffers")
    }

    pub fn current_buffer(&self) -> &Buffer {
        self.buffers
            .get(&self.current_buffer_name)
            .expect("Corruption in the hashmap of buffers")
    }

    pub fn handle_key_event(&mut self, event: KeyEvent, env: &mut Env<EditorState>) {
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

pub fn create_global_env() -> (EditorState, Env<EditorState>) {
    let editor_state = EditorState::new();
    let mut env = Env::new();

    macro_rules! insert_fn {
        ($name:literal, $func:ident) => {
            env.functions.insert($name.into(), LispExp::Primitive(primitives::$func));
        }
    }
    insert_fn!("quit", quit);
    insert_fn!("self-insert", self_insert);
    insert_fn!("insert-newline", insert_newline);
    insert_fn!("backward-char", backward_char);
    insert_fn!("forward-char", forward_char);
    insert_fn!("delete-backward-char", delete_backward_char);
    
    (editor_state, env)
}

// ---------------------------------------------------------------------------//
//                                                                            //
//                                  PRIMITIVES                                //
//                                                                            //
// ---------------------------------------------------------------------------//

mod primitives {
    use crate::lisp::{EvalError, LispExp};
    use super::{EditorState, ELispExp};

    fn is_nil(args: &[ELispExp]) -> bool {
        args.len() == 0 ||
        (args.len() == 1 && (
            args[0] == ELispExp::List(vec![])) ||
            args[0] == ELispExp::Symbol("nil".into()))
    }
    
    macro_rules! nil {
        () => {ELispExp::Symbol("nil".into())}
    }

    macro_rules! primitive {
        ($func_name:ident, $args:ident, $ctx:ident, $body:block) => {
            pub fn $func_name($args: &[ELispExp], $ctx: &mut EditorState) -> Result<ELispExp, EvalError> { $body } } }
    
    primitive!(quit, args, ctx, {
        ctx.running = false;
        Ok(nil!())
    });
    
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
                    got: format!("{:?}", args.first())
            })
        }
    });

    primitive!(insert_newline, args, ctx, {
        let buf = ctx.current_buffer_mut();
        buf.text.insert('\n');
        buf.is_modified = true;
        Ok(LispExp::Symbol("nil".into()))
    });

    primitive!(forward_char, args, ctx, {
        let buf = ctx.current_buffer_mut();
        let current = buf.text.cursor_pos();
        let step = if is_nil(args) { 1 } else {
            if let ELispExp::Number(n) = args[0] {
                n.floor() as usize
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: format!("{:?}", args[0])
                })
            }
        };
        buf.text.move_gap(current + step);

        Ok(nil!())
    });

    primitive!(backward_char, args, ctx, {
        let buf = ctx.current_buffer_mut();
        let current = buf.text.cursor_pos();
        let step = if is_nil(args) { 1 } else {
            if let ELispExp::Number(n) = args[0] {
                n.floor() as usize
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: format!("{:?}", args[0])
                })
            }
        };
        buf.text.move_gap(current - step);

        Ok(nil!())
    });

    primitive!(delete_backward_char, args, ctx, {
        let buf = ctx.current_buffer_mut();
        buf.text.delete();
        buf.is_modified = true;
        Ok(nil!())
    });
}
