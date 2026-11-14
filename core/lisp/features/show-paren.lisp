;;; show-paren --- the bracket next to point, drawn with its partner.
;;;
;;; The minor mode `show-paren-mode', on in every buffer unless turned off:
;;; where it is on, the editor draws the pair in the window with the keyboard.
;;; User manual: the `editing' page.
;;; Design: `doc/architettura-editor.typ, <mod-show-paren>`.

(require 'commands)
(require 'debug)

(define-minor-mode show-paren-mode nil
  "Draw the bracket next to point with the one it pairs with, in the current
buffer: on, off, or the other way with no argument. On in every buffer unless
turned off.

Example:
  (show-paren-mode -1)")

(define-globalized-minor-mode global-show-paren-mode show-paren-mode nil
  "Turn `show-paren-mode' on in every buffer, or off in all of them.
The buffers to come follow; with no argument, the other way from how it is. On
unless turned off." t)

(log "show-paren loaded")
(provide 'show-paren)
