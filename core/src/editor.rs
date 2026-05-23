use std::collections::HashMap;
use crate::buffer::GapBuffer;

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
