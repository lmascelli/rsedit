(make-mode 'minibuffer-mode)

(setq *minibuffer-on-confirm* nil)
(setq *minibuffer-on-change* nil)
(setq *minibuffer-on-cancel* nil)
(setq *minibuffer-previous-buffer* nil)

(defun minibuffer-confirm ()
  "Called when the user presses Enter."
  (let ((input (buffer-string)))
    (close-buffer "*minibuffer*")
    (switch-to-buffer *minibuffer-previous-buffer*)
    (if *minibuffer-on-confirm*
        (funcall *minibuffer-on-confirm* input))))

(defun minibuffer-cancel ()
  "Called when the user presses Escape or C-g."
  (close-buffer *minibuffer*)
  (switch-to-buffer *minibuffer-previous-buffer*)
  (if *minibuffer-on-cancel* (funcall *minibuffer-on-cancel*)))

(defun minibuffer-complete ()
  "Called when the user presses Tab."
  (let ((input (buffer-string)))
    (if *minibuffer-on-change*) (funcall *minibuffer-on-change* input)))

(defun minibuffer-prompt (prompt on-confirm on-change on-cancel)
  "Open a minibuffer with PROMPT.
ON-CONFIRM is a lambda/function taking one string argument.
ON-CHANGE is lambda for Tab-completion."
                                        ; (setq *minibuffer-previous-buffer* (current-buffer))
    (progn
    (setq *minibuffer-on-confirm* on-confirm)
    (setq *minibuffer-on-change* on-change)
    (setq *minibuffer-on-cancel* on-cancel)

    (let ((actual-width (- frame-width 2))
          (actual-height (- frame-height 2)))
      (make-floating-window prompt 1 1 actual-width actual-height))
    ))

(defun command-execute-prompt ()
  (minibuffer-prompt "Command" nil nil nil))

(define-key 'minibuffer-mode "<Return>" 'minibuffer-confirm)
(define-key 'minibuffer-mode "<Escape>"   'minibuffer-cancel)
(define-key 'minibuffer-mode "<Tab>"   'minibuffer-complete)
(define-key nil "M-x" (lambda ()
  (minibuffer-prompt "Command" nil nil nil)))

(log "End of the minibuffer.lisp")
