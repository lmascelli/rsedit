;; Debug helpers.
;;
;; `message' is the everyday way to surface something visibly, right now,
;; from Lisp code you're testing interactively: it shows in the echo area
;; (the status line at the bottom of the frame) *and* stays in the log, so
;; it can be reviewed afterwards via `switch-to-messages' (M-l).
;;
;; `backtrace' and `all-logs' are lower-level: `backtrace' returns the call
;; stack captured at the point of the most recent uncaught error (see the
;; `push_call_frame' docs on `LispContext' in the Rust source for exactly
;; what it can and can't show), and `all-logs' returns the raw log this
;; file's `messages-buffer-refresh' renders into a buffer.

(defun debug--insert-string (s)
  "Insert S into the current buffer, character by character. A small
shared helper -- `self-insert' only takes one character at a time."
  (mapc 'self-insert (split-string s "")))

(defun message (fmt-string &rest args)
  "Format FMT-STRING with ARGS exactly like `format', show the result in
the echo area, and append it to the diagnostic log (see
`switch-to-messages'). Returns the formatted string.

Example:
  (message \"Saved %s\" (current-buffer))"
  (let ((s (apply 'format fmt-string args)))
    (set-echo-message s)
    (log s)
    s))

(defun messages-buffer-refresh ()
  "Rebuild the *Messages* buffer's content from `all-logs' and switch to
it, creating the buffer first if it doesn't exist yet. This is a
snapshot, not a live view -- call again (or M-l) to pick up anything
logged since."
  (buffer-create "*Messages*")
  (switch-to-buffer "*Messages*")
  (clear-buffer)
  (mapc (lambda (line)
          (debug--insert-string line)
          (self-insert "\n"))
        (all-logs)))

(defun switch-to-messages ()
  "Open the *Messages* buffer, showing the diagnostic log so far."
  (messages-buffer-refresh))

(define-key nil "M-l" 'switch-to-messages)

;; `report-error' below is the editor's error-reporting hook: whenever a
;; key-triggered evaluation fails uncaught, the editor calls
;; (report-error MESSAGE FRAMES) if this function is defined -- MESSAGE a
;; string describing the failure, FRAMES the call stack captured at the
;; point of failure (see `backtrace'), innermost call first. Falls back
;; to plain logging if this function isn't (yet) defined, e.g. during
;; early boot.
;;
;; The default implementation below always echoes the error (via
;; `message', so it's visible immediately and stays in the log), and --
;; when `debug-on-error' is non-nil -- also opens a *Backtrace* popup
;; with the full call stack. `debug-on-error' defaults to nil: day-to-day
;; mistakes (a mistyped keybinding, a typo you're actively fixing via
;; M-:) are better served by a quiet echo-area message than an
;; interrupting popup; turn it on while chasing something gnarlier.
;; Redefine `report-error' entirely for a different policy -- e.g. always
;; popping up, or routing errors somewhere else.

(setq debug-on-error nil)

(make-mode 'backtrace-mode)
(setq *backtrace-window-open* nil)
;; The buffer that was current right before the *first* *Backtrace* popup
;; of a (possibly replaced-in-place, see `backtrace-show') sequence
;; opened. `close-floating-window' only restores window *focus*, not
;; `current-buffer' -- see the comment in `backtrace-dismiss' -- so this
;; is tracked explicitly, the same way `*minibuffer-previous-buffer*' is
;; in minibuffer.lisp.
(setq *backtrace-previous-buffer* nil)

(defun backtrace-dismiss ()
  "Close the *Backtrace* window (q or Escape) and switch back to
whatever buffer was current before it opened. `close-floating-window'
alone only restores window focus, not which buffer is \"current\" -- see
`*backtrace-previous-buffer*' -- so that switch is done explicitly here."
  (close-floating-window)
  (setq *backtrace-window-open* nil)
  (if *backtrace-previous-buffer*
      (switch-to-buffer *backtrace-previous-buffer*)))

(define-key 'backtrace-mode "q" 'backtrace-dismiss)
(define-key 'backtrace-mode "<Escape>" 'backtrace-dismiss)

(defun backtrace-show (message frames)
  "Open a *Backtrace* floating window showing MESSAGE and FRAMES (a list
of call-stack frame-name strings, innermost call first, as returned by
`backtrace'). Replaces any *Backtrace* window already open rather than
stacking a new one on top of it. Dismissed with q or Escape."
  (if *backtrace-window-open*
      (close-floating-window)
      ;; Only capture this on the *first* open of a sequence -- on a
      ;; replace, `(current-buffer)' is already \"*Backtrace*\" itself,
      ;; which would clobber the real previous buffer.
      (setq *backtrace-previous-buffer* (current-buffer)))
  (make-floating-window "*Backtrace*" 4 2
                         (max 20 (- frame-width 8))
                         (max 5 (- frame-height 6))
                         "Backtrace (q to dismiss)" 'backtrace-mode)
  (setq *backtrace-window-open* t)
  (debug--insert-string message)
  (insert-newline)
  (if frames
      (mapc (lambda (f)
              (debug--insert-string (format "  at %s" f))
              (insert-newline))
            frames)
      (debug--insert-string
       "  (no call-stack frames captured -- the failure was likely in a tail call; see `backtrace's docstring)")))

(defun report-error (message frames)
  "The editor's default error-reporting hook -- see the comment above.
Always shows MESSAGE via `message'. If `debug-on-error' is non-nil, also
opens a *Backtrace* window with MESSAGE and FRAMES."
  (message "Eval Error: %s" message)
  (if debug-on-error
      (backtrace-show message frames)))

(log "End of the debug.lisp")
