;;; display-line-numbers --- line numbers left of a buffer's text.
;;;
;;; The minor mode `display-line-numbers-mode', off unless turned on: where it
;;; is on, the editor numbers the lines of the windows showing the buffer, as
;;; `display-line-numbers-type' says.
;;; User manual: the `configuration' page.
;;; Design: `doc/architettura-editor.typ, <mod-display-line-numbers>`.

(require 'commands)
(require 'debug)

(define-minor-mode display-line-numbers-mode nil
  "Number the lines of the current buffer, left of its text, in every window
showing it: on, off, or the other way with no argument.

Example:
  (minor-mode-enable 'display-line-numbers-mode 'rust-mode)")

(define-globalized-minor-mode global-display-line-numbers-mode display-line-numbers-mode nil
  "Turn `display-line-numbers-mode' on in every buffer, or off in all of them.
The buffers to come follow; with no argument, the other way from how it is.")

(log "display-line-numbers loaded")
(provide 'display-line-numbers)
