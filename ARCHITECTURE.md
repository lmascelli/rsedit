# rsedit Architecture

rsedit is an extensible, headless text editor engine built in Rust. It draws heavy inspiration from the architectural model of GNU Emacs.

The core philosophy of rsedit is total decoupling: the text manipulation logic, the layout state, and the configuration environment are completely separated from the graphical representation (the frontend). The editor's behavior is entirely programmable via an embedded Lisp interpreter.
1. Core vs. Frontend Boundary

The project is split into two distinct layers:

    rsedit_core (The Engine): A headless library that owns the text buffers, the Lisp Virtual Machine, the cursor state, and the keymaps. It does not know what a "terminal" or a "pixel" is.

    src/tui.rs (The Renderer): A "dumb" frontend client utilizing crossterm. Its only responsibilities are to pass raw keyboard events to the core and to draw a grid of characters exactly where the core tells it to.

Because of this strict boundary, rsedit can theoretically be plugged into a GUI framework (like GTK or wgpu) in the future without changing a single line of the core logic.

2. The Input Pipeline (Symbol Mapping)

Unlike traditional editors that hardcode keystrokes to specific Rust functions (e.g., if key == Up { move_up() }), rsedit uses a Symbol Mapping Strategy.

When a key is pressed, it travels through a specific pipeline:

    Hardware Event: The frontend receives a physical keystroke (e.g., Ctrl+S).

    Abstract KeyEvent: The frontend translates this into a UI-agnostic rsedit_core::input::KeyEvent.

    Keymap Lookup: The core checks the active keymap. It maps Ctrl+S to a string: "save-buffer".

    AST Construction: The core wraps this string into a Lisp Abstract Syntax Tree (AST): (save-buffer). If it was a character like 'a', it builds (self-insert "a").

    Evaluation: The AST is passed to the Lisp eval() function, which locates the native Rust primitive or user-defined Lisp macro and executes it.

This indirection means that every single action in the editor is a Lisp command, making the editor infinitely customizable.

3. State Management

The entire state of the editor lives inside two interconnected structs instantiated at startup:

    EditorState<B: BufferTrait>: Owns the text buffers, tracks the currently focused buffer, manages viewport scrolling variables (scroll_x, scroll_y), and holds the active keymap.

    Env<T>: The lexical environment for the Lisp interpreter. It stores variables (via setq) and functions (via defun or Rust primitives).

Because EditorState is passed as a mutable context (ctx) into every evaluated primitive, native Rust functions can safely mutate the text or layout without violating the borrow checker.

4. The Text Engine

Text manipulation is abstracted behind the BufferTrait. This allows the editor to theoretically swap out the underlying data structure (e.g., changing to a Piece Table or a Rope in the future) without breaking the editing primitives.

Currently, rsedit implements this trait using a GapBuffer.
The Gap Buffer

The GapBuffer maintains a single contiguous Vec<char> with a "gap" (empty space) located exactly where the cursor is.

    Insertion: O(1) time complexity. When the user types, characters are placed directly into the start of the gap.

    Movement: When the cursor moves, the gap is shifted through memory.

Coordinate Translation

The GapBuffer stores characters in a 1-dimensional array. However, editors operate in 2D space (Lines and Columns). To bridge this, the buffer implements highly optimized iterator chains to calculate geometric positions without allocating strings on the heap:

    cursor_1d_to_2d(pos)

    cursor_2d_to_1d(line, col)

This ensures that vertical cursor movement (Up/Down) remains mathematically precise, even when navigating jagged text files with varying line lengths.

5. The Lisp Virtual Machine

rsedit features a custom, fully integrated Lisp interpreter (core/src/lisp.rs) that dictates the editor's behavior.

    Types Supported: Symbols, Strings, Numbers, Lists (), Vectors [], and Maps {}.

    Special Forms: Supports standard Lisp control flows including if, let (with lexical scoping boundaries), progn, defun, and setq.

    Dynamic Evaluation: Map nodes ({}) and Vector nodes ([]) evaluate their inner members dynamically, allowing for complex data structures to be passed into editor configurations.
