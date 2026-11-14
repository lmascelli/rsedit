;;; hl-line --- highlight point's logical line in the selected window.
;;; User manual: the `editing' page.
;;; Design: `doc/architettura-editor.typ, <mod-hl-line>`.
(require 'commands)

(define-minor-mode hl-line-mode "HL"
  "Highlight point's entire logical line in the selected window, including
wrapped rows and their padding. The face hl-line supplies its background.

Example:
  (hl-line-mode 1)")

(define-globalized-minor-mode global-hl-line-mode hl-line-mode nil
  "Turn `hl-line-mode' on in every buffer, or off in all of them.
The buffers to come follow; with no argument, the other way from how it is.")

(provide 'hl-line)
