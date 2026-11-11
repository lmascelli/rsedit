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

;; In every buffer from now on, whatever its major mode, and in those already
;; open, which `minor-mode-enable' leaves as they are.
(minor-mode-enable 'whitespace-mode)
(dolist (name (all-buffer-names))
  (minor-mode-on 'whitespace-mode name))

(log "whitespace loaded")
(provide 'whitespace)
