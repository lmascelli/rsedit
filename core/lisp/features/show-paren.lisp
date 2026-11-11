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

;; In every buffer from now on, whatever its major mode, and in those already
;; open, which `minor-mode-enable' leaves as they are.
(minor-mode-enable 'show-paren-mode)
(dolist (name (all-buffer-names))
  (minor-mode-on 'show-paren-mode name))

(log "show-paren loaded")
(provide 'show-paren)
