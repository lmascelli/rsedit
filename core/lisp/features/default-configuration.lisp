;;; default-configuration --- the editor as it comes: its modules and settings.
;;;
;;; Loads every module the editor ships, and sets what the editor starts with,
;;; so that the init.lisp it writes is one line and what you add to it.
;;; User manual: the `configuration' page.
;;; Design: `doc/architettura-editor.typ, <mod-default-configuration>`.

;; Nothing here may need a macro. Where no module is found -- a test binary
;; with no data/lisp beside it -- each `require' answers nil, and a form
;; written with a macro would be an ordinary call, which fails.

(defvar default-configuration-skip nil
  "The modules `default-configuration' does not load, as a list of their
names; set it before requiring it. A module that one still loaded needs is
loaded anyway.")

(defconst default-configuration-modules
  '(commands
    debug
    common-keymaps
    indent              ; what Tab does; a mode plugs its own rule in
    minibuffer          ; M-:, an expression evaluated at a prompt
    rust-mode           ; colouring for Rust source
    risp-mode           ; colouring and indentation for this editor's own Lisp
    cc-mode             ; colouring and indentation for C/C++ source
    python-mode         ; colouring and indentation for Python
    toml-mode           ; colouring and indentation for TOML
    bash-mode           ; colouring and indentation for Bash
    cmake-mode          ; colouring and indentation for CMake
    dired               ; a directory in a buffer, C-x d
    completion          ; Tab shows every candidate at once, in a strip
    preview             ; C-x r t shows its replacement before you commit to it
    clipboard           ; kills also go to the system clipboard
    electric-pair       ; typing an opening bracket gives you the pair
    show-paren          ; the bracket next to the cursor drawn with its partner
    hl-line             ; highlight the current logical line when enabled
    subword             ; identifier parts for word motion and killing
    whitespace          ; the blanks at the ends of lines, drawn and cleaned
    overwrite           ; Insert, and what you type replaces what is there
    display-line-numbers ; M-x display-line-numbers-mode numbers the lines
    auto-revert         ; a file changed on disk is read again
    fill                ; M-q fills a paragraph, M-x auto-fill-mode as you type
    visual-line         ; M-x visual-line-mode wraps words on screen only
    buffer-list         ; C-x b to switch, C-x C-b for the whole list
    recent-files        ; M-g r to open a file opened before
    shell               ; M-! runs a command and shows what it said
    manpage             ; C-h m, and K on a word
    help                ; C-h f, C-h k, C-h b: the editor describing itself
    risp-repl           ; M-x risp-repl, a buffer where C-j evaluates Lisp
    occur               ; M-s o and M-s g, and the lists M-g n walks
    compile             ; C-c c, and M-g n to walk what it complained about
    theme               ; C-c t to choose how faces are drawn
    vim                 ; M-x vim-mode, modal editing in a buffer
    completion-at-point); the sources C-M-i asks for completions
  "The modules `default-configuration' loads, in this order.")

(dolist (module default-configuration-modules)
  (unless (memq module default-configuration-skip)
    (require module)))

;; Directories first, then files, in a directory listing.
(setq dired-sort 'type)

;; The mouse: click to put point, wheel to scroll the window under the pointer.
;;
;; On here rather than in the editor's own defaults because it costs something.
;; While the editor is reading the mouse the terminal is not, so selecting text
;; with the mouse to paste it into another program stops working -- hold Shift
;; in most terminals to get it back for one drag.
(setq mouse-mode t)

(log "default-configuration loaded")
(provide 'default-configuration)
