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
space, Tab or Return. Return fills the line it just ended. On, off, or the
other way with no argument. `auto-fill-column-show-guide' controls the guide.")

(define-minor-mode auto-fill-column "FillCol"
  "Break lines at `fill-column' after a space, Tab or Return, as
`auto-fill-mode' does. On, off, or toggle with no argument. The guide's
visibility and character are `auto-fill-column-show-guide' and
`auto-fill-column-character'. Either filling mode may be used on its own.")

(defun auto-fill-post-self-insert ()
  "Break the line after a space, Tab or Return past `fill-column'. Run
from `post-self-insert-hook' where `auto-fill-mode' is on."
  (cond
   ((eq this-command 'newline) (do-auto-fill t))
   ((and (> (point) (point-min))
         (or (string= (buffer-substring (- (point) 1) (point)) " ")
             (string= (buffer-substring (- (point) 1) (point)) "\t")))
    (do-auto-fill))))

(add-hook 'auto-fill-mode "post-self-insert-hook" 'auto-fill-post-self-insert)

(defun auto-fill-column--post-self-insert ()
  "Run automatic filling for `auto-fill-column', once if both modes are on."
  (unless (minor-mode-p 'auto-fill-mode)
    (auto-fill-post-self-insert)))

(add-hook 'auto-fill-column "post-self-insert-hook"
          'auto-fill-column--post-self-insert)

(define-key nil "M-q" 'fill-paragraph)
(define-key nil "C-x f" 'set-fill-column)

(log "fill loaded")
(provide 'fill)
