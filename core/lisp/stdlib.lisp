(make-mode "fundamental-mode")
(make-mode "lisp-interactive-mode")

(define-key "C-q" 'quit)
(define-key "C-s" 'save-buffer)
(define-key "<left>" 'backward-char)
(define-key "<right>" 'forward-char)
(define-key "<up>" 'previous-line)
(define-key "<down>" 'next-line)
(define-key "<ret>" 'insert-newline)
(define-key "<backspace>" 'delete-backward-char)

(defun open-config ()
  (find-file "~/.config/rsedit/init.lisp"))
