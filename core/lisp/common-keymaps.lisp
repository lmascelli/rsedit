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

;; Completion at point. C-M-i is Emacs' binding and is the same keystroke as
;; M-<tab>, which is what most terminals actually send -- so both are bound and
;; whichever arrives reaches the same command.
;;
;; What it can complete depends on the buffer: every mode's own sources are
;; tried before the global ones. `completion-functions' lists them and
;; `set-completion-functions' reorders or removes them.
(define-key nil "C-M-i" 'completion-at-point)
(define-key nil "M-tab" 'completion-at-point)

;; Scrolling by a screenful, with two lines of overlap so you can find your
;; place. Point comes along only when the line it is on scrolls out of sight,
;; so reading a long file leaves the cursor where you were looking.
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

;; The region is drawn with the `region' face, which ships bound to reverse
;; video -- no colour to clash with whatever scheme the terminal is set to. To
;; pick colours instead:
;;
;;   (set-face 'region nil "bright-black")         ; a conventional colour
;;   (set-face 'region "#f8f8f2" "#3a5fcd")        ; any colour you like
;;   (set-face 'region nil nil '("reverse"))       ; back to the default
;;
;; A colour is a request. The renderer draws it as asked where it can, and the
;; nearest thing it has where it cannot -- on a sixteen-colour terminal that is
;; a palette slot, which is your own configured colour. `list-colors' names the
;; conventional ones, and they are chosen so they land on the slot of the same
;; name when a terminal has to approximate.
;;
;; `list-faces' names everything that can be styled this way.

;; Prefix key sequences. A binding is written as its keys separated by spaces;
;; pressing the first shows it in the echo area and waits for the rest.
(define-key nil "C-x C-x" 'exchange-point-and-mark)
(define-key nil "C-x h" 'mark-whole-buffer)
(define-key nil "C-x C-f" 'find-file)
(define-key nil "C-x C-s" 'save-buffer)
;; Save as: write somewhere else and visit that from now on. The old file is
;; left alone -- this writes a copy rather than moving anything.
(define-key nil "C-x C-w" 'write-file)
(define-key nil "C-x C-c" 'quit)
(define-key nil "C-x k" 'kill-buffer)

;; Windows. C-x 2 splits above/below, C-x 3 side by side, C-x 0 closes this
;; one, C-x 1 closes the others, C-x o cycles.
(define-key nil "C-x 2" 'split-window-below)
(define-key nil "C-x 3" 'split-window-right)
(define-key nil "C-x 0" 'delete-window)
(define-key nil "C-x 1" 'delete-other-windows)
(define-key nil "C-x o" 'other-window)

;; ... and once you are cycling, a bare `o' keeps going: C-x o o o walks
;; through the windows without the prefix each time. Any other key ends the
;; run and does its own job, so ignoring the offer costs nothing.
(define-repeat-key 'other-window "o")

;; Repeat the last command. The reason this is worth a key of its own is the
;; commands whose binding is long: `C-x u' three times to undo three times is
;; three two-key sequences, where `C-x u C-x z z z' is one sequence and three
;; taps. The repeat key on `repeat' itself is what makes the tail of that work.
(define-key nil "C-x z" 'repeat)
(define-repeat-key 'repeat "z")

;; And bare letters for the two commands that are most often wanted several
;; times running. `C-x z' covers every command including these, but `C-x u'
;; followed by `u u u' is the shape people reach for, and having to type
;; `C-x z' in between to get it is the complaint this answers.
;;
;; The cost of a repeat key is that the letter stops doing its ordinary thing
;; for exactly one keystroke after that command -- and the offer is shown in
;; the frame while it stands, so nobody is left guessing.
(define-repeat-key 'undo "u")
(define-repeat-key 'redo "r")

;; Incremental search. C-s opens a prompt and the buffer jumps to the first
;; match of whatever has been typed so far, re-searching on every keystroke.
;; Inside the search, C-s goes to the next match and C-r turns around; Return
;; stops there, and Escape or C-g puts point back where it started. A search
;; that runs out says so, and repeating it then wraps around the buffer.
;;
;; C-M-s and C-M-r take a regular expression instead.
(define-key nil "C-s" 'isearch-forward)
(define-key nil "C-r" 'isearch-backward)
(define-key nil "C-M-s" 'isearch-forward-regexp)

;; Walking a list of places -- a search's, a compilation's, anything that
;; attached one. Built in rather than a module's, so that whichever list was
;; made last is the one these walk and no producer depends on another's module
;; being loaded.
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

;; Replace. `M-%' is Emacs' key for it and there is no reason to differ.
;;
;;   y   replace this one        !   replace this and all the rest
;;   n   leave it                ^   take back the last one
;;   q   stop
;;
;; Anything else stops and then does what it normally does, so carrying on
;; typing abandons it. In the region when the mark is active, from point
;; onwards otherwise. `M-x replace-string' does the lot without asking.
;;
;; The keys above are the built-in way of asking; `*replace-read-function*'
;; replaces it with something else entirely, which then drives the same
;; `replace-this'/`replace-skip'/`replace-rest' verbs.
(define-key nil "M-%" 'query-replace)
(define-key nil "C-M-%" 'query-replace-regexp)
(define-key nil "C-M-r" 'isearch-backward-regexp)

;; Whether searching ignores case. Emacs' name, Emacs' default -- and it is
;; also set from Rust, so searching behaves the same with no configuration
;; loaded at all. Read by the incremental search and by replace alike; a
;; folded replace still puts the replacement in with the casing of what it
;; replaced, which is `case-replace'.
(setq case-fold-search t)

;; Motion over balanced expressions: the `editing' manual page.
(define-key nil "C-M-f" 'forward-sexp)
(define-key nil "C-M-b" 'backward-sexp)
(define-key nil "C-M-k" 'kill-sexp)
(define-key nil "C-M-<backspace>" 'backward-kill-sexp)
(define-key nil "C-M-u" 'backward-up-list)
(define-key nil "C-M-d" 'down-list)

;; `up-list' is left unbound, as in Emacs.

;; Put the line point is on in the middle of the window, without moving point.
;; The text slides under the cursor; `C-v' moves the cursor with the text.
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

;; Files that change underneath you. A background job compares each open
;; file's timestamp and length against what they were when the editor last read
;; or wrote it, a few files at a time:
;;
;;   (setq watch-files nil)          ; off; t or unset is on
;;   (setq watch-file-interval 10)   ; seconds; read once, at startup
;;
;; A buffer with nothing unsaved is reloaded silently -- switch branches and
;; the windows are already showing the new text. A buffer with unsaved edits is
;; never touched: it is marked, and `C-x C-s' asks before writing over whatever
;; arrived. `M-x revert-buffer' rereads deliberately, asking first if there is
;; anything to lose.
;;
;; Turn it off if a network filesystem is misbehaving -- the check is a `stat',
;; and a `stat' on a dead mount blocks the thread that also colours syntax.

;; Work that has never been saved. Every half minute, a copy of each modified
;; file buffer is written beside it as `#name#' -- not a backup of the file,
;; but a copy of what is in the buffer, for the one loss nothing else covers:
;; the editor stopping without being asked.
;;
;;   (setq auto-save nil)                      ; off; t or unset is on
;;   (setq auto-save-interval 60)              ; seconds; read once, at startup
;;   (setq auto-save-directory "~/.cache/rsedit") ; all of them here instead
;;
;; With a directory set, the file's path is flattened into the name
;; (`#!home!me!p!main.rs#') so that two `main.rs' from different directories do
;; not share one copy.
;;
;; Opening a file that has one says so. `M-x recover-file' puts it in the
;; buffer and marks it modified, so you can look before deciding: `C-x C-s'
;; keeps it and deletes the copy, `M-x revert-buffer' throws it away.

;; Background workers. `define-worker' runs a fiber on the scheduler syntax
;; colouring runs on, resuming it once per step:
;;
;;   (define-worker 'indexer
;;     (fiber (while t
;;              (if (index--stale-p)
;;                  (progn (index--one-chunk) (yield 'continue))
;;                (yield '(wait buffer-changed))))))
;;
;; The `yield' is the whole contract: it is where the worker is willing to be
;; put down, and everything between two of them is one step's work. What it
;; yields says when the next step comes -- at once, after a sleep, or when an
;; event it names is emitted; a plain (yield) waits 40 ms. See `define-worker'
;; for the table. A step has its own budget -- `worker-fuel' -- separate from
;; the one commands run under, so nothing you type can cut a worker short and
;; no worker can spend your budget. A step that never reaches a `yield' is
;; stopped and the worker with it, because the thread it holds is the one
;; colouring runs on.
;;
;; `M-x list-workers' says which are running; `stop-worker' stops one, and
;; `(work-list)' shows everything the scheduler is running, the editor's own
;; work included. Defining a worker over an existing name replaces it, so
;; re-evaluating a module leaves one worker rather than two.
(defun list-workers ()
  "Show which background workers are running."
  (let ((names (running-workers)))
    (if (null names)
        (message "No workers running")
        (message "Workers: %s" names))))
(register-command "list-workers" nil)

(log "End of the common-keymaps.lisp")
