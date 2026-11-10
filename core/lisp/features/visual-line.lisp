;;; visual-line --- word wrapping without changing the buffer's text.
;;; User manual: the `wrapping' page.
;;; Design: `doc/architettura-editor.typ, <righe-visive>`.
(require 'commands)
(require 'debug)

(define-minor-mode visual-line-mode "Wrap"
  "Wrap long lines at word boundaries in the current buffer, without inserting
newlines. With no argument toggle; a positive argument enables the mode,
zero or a negative argument disables it. Movement follows visual rows.

Example:
  (visual-line-mode 1)")

(define-key 'visual-line-mode "<up>" 'previous-visual-line)
(define-key 'visual-line-mode "<down>" 'next-visual-line)
(define-key 'visual-line-mode "C-p" 'previous-visual-line)
(define-key 'visual-line-mode "C-n" 'next-visual-line)
(define-key 'visual-line-mode "<home>" 'beginning-of-visual-line)
(define-key 'visual-line-mode "<end>" 'end-of-visual-line)
(define-key 'visual-line-mode "C-a" 'beginning-of-visual-line)
(define-key 'visual-line-mode "C-e" 'end-of-visual-line)

(provide 'visual-line)
