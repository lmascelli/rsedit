(define-key nil "<backspace>" 'delete-backward-char)

;; Movement. Emacs bindings where a single key sequence can express them --
;; C-a / C-e / M-f / M-b / M-< / M-> all fit; anything needing a prefix key
;; (C-x style) waits on multi-key sequences.
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

;; Paragraph motion was on M-n and M-p, which is not what those keys do in
;; Emacs -- they are not globally bound at all there, and mean "next/previous
;; history entry" in a prompt. M-} and M-{ are the real bindings.

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

;; Deletion, mirroring the movement bindings above. These discard the text
;; rather than saving it -- the kill ring is roadmap #20 -- but they carry the
;; Emacs names so the bindings will not change once it exists.
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

;; Redo was on M-_, which Emacs does not use. `C-?' is what Emacs 28 gave
;; `undo-redo', so that is where it goes -- with the caveat that some terminals
;; cannot send it. On one of those, reach redo through M-x until you bind it to
;; something your terminal does send.
(define-key nil "C-?" 'redo)

;; The mark and the region. C-<space> sets the mark; moving point from there
;; grows the region, and any edit ends it.
(define-key nil "C-<space>" 'set-mark)

;; The kill ring. C-w and M-w act on the region; C-y puts the most recent kill
;; back and M-y walks further into the ring, but only straight after a yank.
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

;; The small verbs: case, whitespace, lines, transposition, zapping. Emacs'
;; keys, because these are the ones most fingers already know -- and because
;; every one of them is a verb rather than a mode, so there is nothing to learn
;; beyond where it is.
;;
;; `duplicate-line' is left unbound: it is wanted often enough to exist and not
;; often enough to spend a key on, and `C-c' is yours to put it under.
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

;; Commenting. `M-;' comments the region out, or the line when there is no
;; region, and takes the comments off again when everything in the scope is
;; already one. What a comment looks like comes from the mode -- see
;; `set-comment-syntax' -- so this key does the right thing in Rust, C and this
;; editor's own Lisp without knowing about any of them.
;;
;; `comment-indent' starts a comment *beside* the code rather than instead of
;; it, and is left unbound: it is wanted far less often than the toggle, and
;; the key most fingers reach for is this one.
(define-key nil "M-;" 'comment-dwim)
(define-key nil "C-x C-;" 'comment-line)

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

;; Structural motion, over balanced expressions rather than over words. What
;; counts as a delimiter, a string or a comment comes from the buffer's mode 
;; see `set-syntax-pairs'. A mode that declared nothing still gets brackets,
;; double quotes and backslash, so these work everywhere.
(define-key nil "C-M-f" 'forward-sexp)
(define-key nil "C-M-b" 'backward-sexp)
(define-key nil "C-M-k" 'kill-sexp)
(define-key nil "C-M-<backspace>" 'backward-kill-sexp)
(define-key nil "C-M-u" 'backward-up-list)
(define-key nil "C-M-d" 'down-list)

;; `up-list' is left unbound, as in Emacs -- C-M-u is the *backward* one there,
;; and reaching the forward one through M-x is what people expect.

;; Put the line point is on in the middle of the window, without moving point.
;; The text slides under the cursor; `C-v' moves the cursor with the text.
(define-key nil "C-l" 'recenter)

;; Tab: the region, or the line, or whatever `tab-always-indent' says when the
;; line is already indented. See indent.lisp.
(define-key nil "tab" 'indent-for-tab-command)

;; C-g abandons a half-typed key sequence or prefix argument, and ends the
;; region. It does not interrupt a running command -- that is roadmap #24.
(define-key nil "C-g" 'keyboard-quit)

;; Each window carries a status line on its bottom row. `mode-line-format' is
;; the string it shows, with Emacs' escapes:
;;
;;   %b buffer   %f file path   %m major mode
;;   %l line     %c column      %p where point is (All/Top/Bot/NN%)
;;   %* "**" when modified, "--" when not      %% a literal per cent
;;
;;   (setq mode-line-format " %* %b   %m   L%l C%c   %p ")   ; the default
;;
;; It is styled with the `mode-line' face, and `mode-line-inactive' for a
;; window that does not have focus -- see `set-face'.

;; Line numbers down the left edge of every tiled window.
;;
;;   (setq display-line-numbers t)          ; each line's own number
;;   (setq display-line-numbers 'relative)  ; how far each line is from point
;;   (setq display-line-numbers nil)        ; off, which is the default
;;
;; `display-line-numbers-width' is the fewest digits the gutter reserves --
;; three unless it is set -- so the text does not shuffle sideways as a file
;; grows past its ninth line and then its ninety-ninth. It widens past that
;; when a number needs the room.
;;
;; The numbers use the `line-number' face, and the line point is on uses
;; `line-number-current'. A window too narrow to spare the columns shows no
;; numbers at all: the text wins.
;;
;; Bound to nothing here, and off, because a gutter costs columns and that is
;; not a trade to make on somebody's behalf. `M-x toggle-line-numbers' turns
;; it on for a look.
(setq display-line-numbers nil)

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

;; Background workers. `define-worker' runs a fiber on the same timer syntax
;; colouring runs on, resuming it once per turn:
;;
;;   (define-worker 'indexer
;;     (fiber (while t
;;              (if (index--stale-p) (index--one-chunk))
;;              (yield))))
;;
;; The `yield' is the whole contract: it is where the worker is willing to be
;; put down, and everything between two of them is one turn's work. A turn has
;; its own budget -- `worker-fuel' -- separate from the one commands run under,
;; so nothing you type can cut a worker short and no worker can spend your
;; budget. A turn that never reaches a `yield' is stopped and the worker
;; retired, because the thread it holds is the one colouring runs on.
;;
;; `M-x list-workers' says which are running; `stop-worker' retires one.
;; Defining a worker over an existing name replaces it, so re-evaluating a
;; module leaves one worker rather than two.
(defun list-workers ()
  "Show which background workers are running."
  (let ((names (running-workers)))
    (if (null names)
        (message "No workers running")
        (message "Workers: %s" names))))
(register-command "list-workers" nil)

(log "End of the common-keymaps.lisp")
