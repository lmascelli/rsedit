(make-mode 'fundamental-mode)
(make-mode 'lisp-interactive-mode)

(define-key nil "C-s" 'save-buffer)
(define-key nil "<backspace>" 'delete-backward-char)

(defun open-config ()
  (find-file "~/.config/rsedit/init.lisp"))

(eval-file "core/lisp/minibuffer.lisp")
(log "End of the stdlib.lisp")
