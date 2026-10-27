;;; debug --- messages, the log, and what happens when a command fails.
;;;
;;; `message' shows a line in the echo area and keeps it in the log,
;;; `switch-to-messages' shows the log, and `report-error' is what the editor
;;; calls when a command fails. User manual: the `diagnostics' page.

(defun message (fmt-string &rest args)
  "Show FMT-STRING, formatted with ARGS as `format' does, in the echo area, and
add it to the log. Return the formatted string.

Example:
  (message \"Saved %s\" (current-buffer))"
  (let ((s (apply 'format fmt-string args)))
    (set-echo-message s)
    (log s)
    s))

(defun messages-buffer-refresh ()
  "Show the log, as it is now, in the *Messages* buffer."
  (buffer-create "*Messages*")
  (switch-to-buffer "*Messages*")
  (clear-buffer)
  (mapc (lambda (line) (insert line "\n")) (all-logs)))

;; A `defun' and a `register-command' rather than `defcommand', so that this
;; file loads without commands.lisp before it.
(defun switch-to-messages ()
  "Show everything the editor has logged, in the *Messages* buffer. The buffer
is a snapshot: run this again to see what came after."
  (messages-buffer-refresh))
(register-command "switch-to-messages" nil)

(defvar debug-on-error nil
  "Whether a failed command also opens a *Backtrace* window with its call
stack, besides the message in the echo area.")

(make-mode 'backtrace-mode)
(setq *backtrace-window-open* nil)
;; The buffer to go back to when the window closes: `close-floating-window'
;; gives back the focus, not the current buffer.
(setq *backtrace-previous-buffer* nil)

(defun backtrace-dismiss ()
  "Close the *Backtrace* window and go back to the buffer that was current
before it opened."
  (close-floating-window)
  (setq *backtrace-window-open* nil)
  (if *backtrace-previous-buffer*
      (switch-to-buffer *backtrace-previous-buffer*)))

(define-key 'backtrace-mode "q" 'backtrace-dismiss)
(define-key 'backtrace-mode "<Escape>" 'backtrace-dismiss)

(defun backtrace-show (message frames)
  "Show MESSAGE and the call stack FRAMES, innermost call first, in a
*Backtrace* window, in place of the one already open."
  (if *backtrace-window-open*
      (close-floating-window)
      ;; Only for the first window: when one replaces another, the current
      ;; buffer is the *Backtrace* itself.
      (setq *backtrace-previous-buffer* (current-buffer)))
  (make-floating-window "*Backtrace*" 4 2
                         (max 20 (- frame-width 8))
                         (max 5 (- frame-height 6))
                         "Backtrace (q to dismiss)" 'backtrace-mode)
  (setq *backtrace-window-open* t)
  (insert message "\n")
  (if frames
      (mapc (lambda (f) (insert "  at " f "\n")) frames)
      (insert "  (no frames: the call that failed was the last one in its function)")))

(defun report-error (message frames)
  "Report a failed command: show MESSAGE in the echo area and, when
`debug-on-error' is set, its call stack FRAMES in a *Backtrace* window.

The editor calls this when a command fails; redefine it to report failures
another way."
  (message "Eval Error: %s" message)
  (if debug-on-error
      (backtrace-show message frames)))

(log "End of the debug.lisp")
(provide 'debug)
