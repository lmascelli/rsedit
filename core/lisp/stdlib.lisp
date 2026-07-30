(make-mode 'fundamental-mode)
(make-mode 'lisp-interactive-mode)

(defun minibuffer-toggle ()
  (make-floating-window "*minibuffer*" 1 27 50 3 " ")
  )

(define-key nil "M-x" 'minibuffer-toggle)
(define-key nil "<backspace>" 'delete-backward-char)

(defun open-config ()
  (find-file "~/.config/rsedit/init.lisp"))

(eval-file "minibuffer")
(log "End of the stdlib.lisp")
