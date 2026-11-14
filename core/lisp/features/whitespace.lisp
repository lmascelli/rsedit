;;; whitespace --- the blanks at the ends of lines, drawn and cleaned.
;;;
;;; Two minor modes: `whitespace-mode' draws the spaces and Tabs that end a
;;; line, on in every buffer unless turned off, and `whitespace-cleanup-mode'
;;; deletes them each time the buffer is saved, off unless turned on.
;;; User manual: the `editing' page.
;;; Design: `doc/architettura-editor.typ, <mod-whitespace>`.

(require 'commands)
(require 'debug)

(define-minor-mode whitespace-mode nil
  "Draw the spaces and Tabs at the ends of lines in the current buffer, when
it visits a file: on, off, or the other way with no argument. On in every
buffer unless turned off.

Example:
  (whitespace-mode -1)")

(define-minor-mode whitespace-cleanup-mode "Clean"
  "Delete the spaces and Tabs at the ends of lines each time the current
buffer is saved, as `delete-trailing-whitespace' cleans a whole buffer: on,
off, or the other way with no argument.

Example:
  (minor-mode-enable 'whitespace-cleanup-mode 'rust-mode)")

(defun whitespace--cleanup ()
  "Clean the whole buffer before saving, including when a region is active."
  (delete-trailing-whitespace (point-min) (point-max)))

(add-hook 'whitespace-cleanup-mode "before-save-hook" 'whitespace--cleanup)

(define-globalized-minor-mode global-whitespace-mode whitespace-mode nil
  "Turn `whitespace-mode' on in every buffer, or off in all of them.
The buffers to come follow; with no argument, the other way from how it is. On
unless turned off." t)

(log "whitespace loaded")
(provide 'whitespace)
