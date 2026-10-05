;; Commands built on top of the built-in minibuffer (see
;; `core/src/feature/minibuffer.rs` -- the read/confirm/cancel/complete mechanics
;; themselves are hardcoded in Rust, not defined here).

;; M-x lives in commands.lisp, which defines the command registry's Lisp
;; side; the stub that used to sit here opened a prompt whose Return did
;; nothing, because there was no dispatch to hand the name to.

(defun eval-expression-confirm (input)
  "Called when the user presses Return after `eval-expression-prompt' --
evaluate INPUT (a string) as a Lisp expression and show INPUT alongside
its result (or, if evaluation failed, alongside a description of the
error) via `message', so it's visible right away and stays in the log
for `switch-to-messages'.
Uses `eval-string-safe' rather than `eval-string' so a bad expression --
a typo, an unbound variable, wrong argument counts -- is reported instead
of aborting the confirm flow."
  (let ((outcome (eval-string-safe input)))
    (if (car outcome)
        (message "%s => %s" input (nth 1 outcome))
        (message "%s !! %s" input (nth 1 outcome)))))

(defun eval-expression-prompt ()
  "Read a Lisp expression via the minibuffer (M-:) and evaluate it in the
running editor, showing the result in the echo area. Useful for testing
code interactively."
  (minibuffer-read "Eval:" 'eval-expression-confirm nil nil))

(define-key nil "M-:" 'eval-expression-prompt)

(log "End of the minibuffer.lisp")
