# rsedit Lisp Reference Manual

The heart of `rsedit` is its embedded Lisp interpreter. The editor's state, configuration, and actions are manipulated by evaluating Lisp Abstract Syntax Trees (ASTs) against a dynamic environment.

## 1. Data Types

The lexer and parser support a rich set of data structures, allowing for complex configuration definitions.

* **Numbers:** Standard 64-bit floats (`42`, `-3.14`, `0.5`).
* **Strings:** Escaped string literals (`"Hello World"`, `"path/to/file.txt"`).
* **Symbols:** Identifiers for variables and functions (`my-var`, `+`, `save-buffer`).
* **Lists `(...)`:** The core executable structure. Evaluated as a function call unless quoted.
* **Vectors `[...]`:** Contiguous arrays. Elements are dynamically evaluated. Example: `[1 (+ 1 1) 3]` evaluates to `[1.0, 2.0, 3.0]`.
* **Maps `{...}`:** Key-Value hash maps. Keys must be strings or symbols; values are dynamically evaluated. Example: `{ "name" "rsedit" "version" 1.0 }`.

---

## 2. Special Forms

Special forms are built-in Lisp structures that do not follow standard function evaluation rules (e.g., they conditionally evaluate their arguments or manage variable scoping).

### `(if condition true_branch [false_branch])`
Evaluates the `condition`. If it is truthy, it evaluates and returns `true_branch`. Otherwise, it evaluates and returns `false_branch` (or `nil` if omitted).
> **Note on Truthiness:** In `rsedit`, only the symbol `nil` and the empty list `()` are considered false. Everything else (including `0`, `""`, and `[]`) is true.

### `(setq symbol value [symbol value ...])`
Assigns a value to a variable in the current environment. Can chain multiple assignments.

```lisp
(setq tab-width 4 indent-tabs-mode nil)
```

```lisp
(defun name (params...) body)
```

Defines a new function in the global environment's function namespace. It binds the body as a lambda.

```lisp
(defun duplicate-line ()
  (progn 
    ; Implementation here...
  ))
```

```lisp
(let ((var1 val1) (var2 val2)) body...)
```

Creates a new lexical environment frame, evaluates the variable bindings, and executes the body. Local variables shadow global variables and are safely discarded after execution.

```lisp
(let ((x 10)
      (y 20))
  (+ x y)) ; Evaluates to 30. x and y are unbound afterward.
```

```lisp
(progn body...)
```

Evaluates a sequence of expressions in order, returning the value of the final expression. Essential for executing multiple side-effects inside an if branch or a lambda.

3. Native Editing Primitives

These are high-performance Rust functions exposed to the Lisp environment via create_global_env(). They interact directly with the editor's BufferTrait and layout context.
Basic Editing

    (self-insert "c"): Inserts a character string into the currently focused buffer at the cursor position.

    (insert-newline): Inserts a \n character.

    (delete-backward-char): Deletes the character immediately preceding the cursor.

## Cursor Movement

    (forward-char [n]): Moves the cursor forward by n characters (defaults to 1).

    (backward-char [n]): Moves the cursor backward by n characters (defaults to 1). Stops safely at the beginning of the buffer.

    (next-line): Moves the cursor geometrically down one line, maintaining horizontal column constraints where possible.

    (previous-line): Moves the cursor geometrically up one line.

## File I/O & System

    (find-file "path/to/file"): Reads a file from disk into a new buffer, registers it in the state, and switches focus to it.

    (save-buffer): Writes the contents of the currently focused buffer back to its associated file path.

    (quit): Gracefully terminates the editor engine loop.
