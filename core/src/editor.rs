use std::collections::HashMap;
use crate::buffer::GapBuffer;
use crate::lisp::{Env, EvalError, LispExp};
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

        Self {
            buffers,
            current_buffer_name: scratch_name,
            echo_message: "Welcome to rsedit".to_string(),
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
}

pub fn create_env() -> (EditorState, Env<EditorState>) {
    let editor_state = EditorState::new();
    let mut env = Env::new();

    macro_rules! insert_fn {
        ($name:literal, $func:ident) => {
            env.functions.insert($name.into(), LispExp::Primitive(primitives::$func));
        }
    }
    insert_fn!("quit", quit);
    
    (editor_state, env)
}

// ---------------------------------------------------------------------------//
//                                                                            //
//                                  PRIMITIVES                                //
//                                                                            //
// ---------------------------------------------------------------------------//

mod primitives {
    use crate::lisp::{Env, EvalError, LispExp};
    use super::{EditorState, ELispExp};
    
    macro_rules! nil {
        () => {ELispExp::Symbol("nil".into())}
    }
    
    pub fn quit(args: &[ELispExp], ctx: &mut EditorState) -> Result<ELispExp, EvalError> {
        ctx.running = false;
        Ok(nil!())
    }
}
