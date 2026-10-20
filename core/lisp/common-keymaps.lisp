(define-key nil "<backspace>" 'delete-backward-char)

;; Moving, typing and the small verbs: the `editing' manual page.
(define-key nil "C-a" 'beginning-of-line)
(define-key nil "C-e" 'end-of-line)
(define-key nil "C-f" 'forward-char)
(define-key nil "C-b" 'backward-char)
(define-key nil "C-n" 'next-line)
(define-key nil "C-p" 'previous-line)
(define-key nil "M-f" 'forward-word)
(define-key nil "M-b" 'backward-word)
(define-key nil "M-}" 'forward-paragraph)
(define-key nil "M-{" 'backward-paragraph)
(define-key nil "M-<" 'beginning-of-buffer)
(define-key nil "M->" 'end-of-buffer)

(define-key nil "C-o" 'open-line-above)

;; Going to a line. M-g is a prefix in Emacs, and both of the keys after it
;; reach the same command, so both are bound.
(define-key nil "M-g g" 'goto-line)
(define-key nil "M-g M-g" 'goto-line)

;; Completion at point: the `completion' manual page. C-M-i is the same
;; keystroke as M-<tab> on most terminals, so both are bound.
(define-key nil "C-M-i" 'completion-at-point)
(define-key nil "M-tab" 'completion-at-point)

;; Scrolling: the `windows' manual page.
(define-key nil "C-v" 'scroll-up-command)
(define-key nil "M-v" 'scroll-down-command)

;; Deleting and killing: the `editing' and `killing' manual pages.
;; electric-pair.lisp takes Backspace and C-d over when it is loaded.
(define-key nil "C-d" 'delete-char)
(define-key nil "C-k" 'kill-line)
(define-key nil "M-d" 'kill-word)
(define-key nil "M-<backspace>" 'backward-kill-word)

;; Undo and redo. C-/ and C-_ are the same keystroke on most terminals -- the
;; terminal reports C-/ as C-_ -- so both are bound, and whichever one the
;; terminal sends arrives at the same command.
(define-key nil "C-/" 'undo)
(define-key nil "C-_" 'undo)
(define-key nil "C-x u" 'undo)

;; Redo on `C-?', where Emacs 28 put `undo-redo'. Some terminals cannot send
;; it: the `editing' manual page says what to do then.
(define-key nil "C-?" 'redo)

;; The mark, the region and the kill ring: the `killing' manual page.
(define-key nil "C-<space>" 'set-mark)
(define-key nil "C-w" 'kill-region)
(define-key nil "M-w" 'kill-ring-save)
(define-key nil "C-y" 'yank)
(define-key nil "M-y" 'yank-pop)

;; The region is drawn in the face region, reverse video unless a theme says
;; otherwise: the `themes' manual page says how to change a face.

;; Keys, and sequences of them: the `keymaps' manual page.
(define-key nil "C-x C-x" 'exchange-point-and-mark)
(define-key nil "C-x h" 'mark-whole-buffer)

;; Files and buffers: the `files' and `buffers' manual pages.
(define-key nil "C-x C-f" 'find-file)
(define-key nil "C-x C-s" 'save-buffer)
(define-key nil "C-x C-w" 'write-file)
(define-key nil "C-x C-c" 'quit)
(define-key nil "C-x k" 'kill-buffer)

;; Windows: the `windows' manual page. After C-x o, a bare o goes on.
(define-key nil "C-x 2" 'split-window-below)
(define-key nil "C-x 3" 'split-window-right)
(define-key nil "C-x 0" 'delete-window)
(define-key nil "C-x 1" 'delete-other-windows)
(define-key nil "C-x o" 'other-window)

(define-repeat-key 'other-window "o")

;; Repeating: the `keymaps' manual page. C-x z repeats any command, and undo
;; and redo, often wanted several times running, have repeat keys of their own.
(define-key nil "C-x z" 'repeat)
(define-repeat-key 'repeat "z")
(define-repeat-key 'undo "u")
(define-repeat-key 'redo "r")

;; Searching, and walking a list of places: the `searching' manual page.
(define-key nil "C-s" 'isearch-forward)
(define-key nil "C-r" 'isearch-backward)
(define-key nil "C-M-s" 'isearch-forward-regexp)

(define-key nil "M-g n" 'next-error)
(define-key nil "M-g p" 'previous-error)

;; The small verbs, on Emacs' keys: the `editing' manual page.
;; `duplicate-line' is left unbound, wanted too seldom to spend a key on.
(define-key nil "M-u" 'upcase-word)
(define-key nil "M-l" 'downcase-word)
(define-key nil "M-c" 'capitalize-word)
(define-key nil "M-\\" 'delete-horizontal-space)
(define-key nil "M-<space>" 'just-one-space)
(define-key nil "C-x C-o" 'delete-blank-lines)
(define-key nil "M-m" 'back-to-indentation)
(define-key nil "M-^" 'join-line)
(define-key nil "C-t" 'transpose-chars)
(define-key nil "M-t" 'transpose-words)
(define-key nil "C-x C-t" 'transpose-lines)
(define-key nil "M-z" 'zap-to-char)

;; Comments: the `editing' manual page. `comment-indent' is left unbound:
;; the key most fingers reach for is the toggle.
(define-key nil "M-;" 'comment-dwim)
(define-key nil "C-x C-;" 'comment-line)

;; Keyboard macros: the `macros' manual page.
(define-key nil "C-x (" 'kmacro-start-macro)
(define-key nil "C-x )" 'kmacro-end-macro)
(define-key nil "C-x e" 'kmacro-end-and-call-macro)
(define-key nil "C-x C-k C-i" 'kmacro-insert-counter)
(define-key nil "C-x C-k C-c" 'kmacro-set-counter)

;; Rectangles: the `rectangles' manual page. `C-x r' is where Emacs keeps
;; them, and its registers too; the letters registers use are free here.
(define-key nil "C-x <space>" 'rectangle-mark-mode)
(define-key nil "C-x r k" 'kill-rectangle)
(define-key nil "C-x r d" 'delete-rectangle)
(define-key nil "C-x r M-w" 'copy-rectangle-as-kill)
(define-key nil "C-x r y" 'yank-rectangle)
(define-key nil "C-x r o" 'open-rectangle)
(define-key nil "C-x r c" 'clear-rectangle)
(define-key nil "C-x r t" 'string-rectangle)

;; Replacing, and `case-fold-search': the `searching' manual page.
(define-key nil "M-%" 'query-replace)
(define-key nil "C-M-%" 'query-replace-regexp)
(define-key nil "C-M-r" 'isearch-backward-regexp)

;; Motion over balanced expressions: the `editing' manual page.
(define-key nil "C-M-f" 'forward-sexp)
(define-key nil "C-M-b" 'backward-sexp)
(define-key nil "C-M-k" 'kill-sexp)
(define-key nil "C-M-<backspace>" 'backward-kill-sexp)
(define-key nil "C-M-u" 'backward-up-list)
(define-key nil "C-M-d" 'down-list)

;; `up-list' is left unbound, as in Emacs.

;; C-l: the `windows' manual page.
(define-key nil "C-l" 'recenter)

;; Tab and Shift-Tab: the `indentation' manual page. Return is bound in
;; indent.lisp, beside the command it runs.
(define-key nil "tab" 'indent-for-tab-command)
(define-key nil "S-<tab>" 'indent-rigidly-left)

;; C-g: the `editing' manual page. It does not interrupt a running command:
;; roadmap #24.
(define-key nil "C-g" 'keyboard-quit)

;; The mode line and the line numbers: `mode-line-format' and
;; `display-line-numbers', documented where `C-h v' finds them, and the
;; `configuration' manual page. Line numbers are off and bound to no key,
;; because they cost columns; `M-x toggle-line-numbers' turns them on.

;; Written out as a `defun' plus a `register-command' rather than with
;; `defcommand', because this file is loaded on its own by tests and by an
;; installation with `commands.lisp' commented out. Where that macro is
;; missing a form using it is not a definition, it is a call -- and the call
;; raises, taking the rest of this file with it.
(defun toggle-line-numbers ()
  "Turn the line-number gutter on, or off if it is already on."
  (setq display-line-numbers (if display-line-numbers nil t))
  (message (if display-line-numbers "line numbers on" "line numbers off")))
(register-command "toggle-line-numbers" nil)

;; Watching files and auto-saving: `watch-files' and `auto-save', documented
;; where `C-h v' finds them, and the `files' manual page.

;; Background workers: the `background' manual page.
(defun list-workers ()
  "Show which background workers are running."
  (let ((names (running-workers)))
    (if (null names)
        (message "No workers running")
        (message "Workers: %s" names))))
(register-command "list-workers" nil)

(log "End of the common-keymaps.lisp")
