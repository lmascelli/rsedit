(make-mode 'fundamental-mode)
(make-mode 'lisp-interactive-mode)

(define-key nil "<backspace>" 'delete-backward-char)
(eval-file "minibuffer")

(defun open-config ()
  (find-file "~/.config/rsedit/init.lisp"))

(log "End of the stdlib.lisp")
