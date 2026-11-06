;;; fill --- paragraphs kept to `fill-column', and lines broken as you type.
;;;
;;; M-q fills the paragraph, or the comment, point is in, C-x f sets the
;;; column, and the minor mode `auto-fill-mode' breaks lines as you type past
;;; it. User manual: the `filling' page.
;;; Design: `doc/architettura-editor.typ, <mod-fill>`.

(require 'commands)
(require 'debug)

(define-minor-mode auto-fill-mode "Fill"
  "Break lines as you type past `fill-column' in the current buffer, after a
space: on, off, or the other way with no argument.")

(defun auto-fill-post-self-insert ()
  "Break the line when a space has just been typed past `fill-column'. Run
from `post-self-insert-hook' where `auto-fill-mode' is on."
  (if (and (> (point) (point-min))
           (string= (buffer-substring (- (point) 1) (point)) " "))
      (do-auto-fill)))

(add-hook 'auto-fill-mode "post-self-insert-hook" 'auto-fill-post-self-insert)

(define-key nil "M-q" 'fill-paragraph)
(define-key nil "C-x f" 'set-fill-column)

(log "fill loaded")
(provide 'fill)
