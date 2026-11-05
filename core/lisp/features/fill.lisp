;;; fill --- paragraphs kept to `fill-column', and lines broken as you type.
;;;
;;; M-q fills the paragraph, or the comment, point is in, C-x f sets the
;;; column, and M-x auto-fill-mode breaks lines as you type past it, in one
;;; buffer or in every buffer of a mode. User manual: the `filling' page.
;;; Design: `doc/architettura-editor.typ, <mod-fill>`.

(require 'commands)
(require 'debug)

(defun auto-fill-mode-p ()
  "Return t when lines are broken as you type in the current buffer."
  (if (buffer-get 'auto-fill) t nil))

(defcommand auto-fill-mode () nil
  "Break lines as you type past `fill-column' in the current buffer, or stop
when it does already."
  (buffer-put 'auto-fill (if (auto-fill-mode-p) nil t))
  (message (if (auto-fill-mode-p) "Auto fill on" "Auto fill off")))

(defun auto-fill-post-self-insert ()
  "Break the line when a space has just been typed past `fill-column', in a
buffer where `auto-fill-mode' is on. Run from `post-self-insert-hook'."
  (if (and (auto-fill-mode-p)
           (> (point) (point-min))
           (string= (buffer-substring (- (point) 1) (point)) " "))
      (do-auto-fill)))

(add-hook nil "post-self-insert-hook" 'auto-fill-post-self-insert)

(defun auto-fill--turn-on ()
  "Turn `auto-fill-mode' on in the current buffer."
  (buffer-put 'auto-fill t))

(defun auto-fill-enable (mode)
  "Turn `auto-fill-mode' on in every buffer of MODE made from now on.

Example:
  (auto-fill-enable 'fundamental-mode)"
  (add-hook mode "major-mode-hook" 'auto-fill--turn-on))

(define-key nil "M-q" 'fill-paragraph)
(define-key nil "C-x f" 'set-fill-column)

(log "fill loaded")
(provide 'fill)
