;;; subword --- word motion and killing at identifier boundaries.
;;; User manual: the `editing' page.
;;; Design: `doc/architettura-editor.typ, <mod-subword>`.
(require 'commands)

(defvar subword-separators "_"
  "Literal characters that split identifiers in subword-mode. An empty string
keeps underscores and hyphens inside identifiers; other punctuation still
separates words. Set a buffer override with buffer-put.

Example:
  (setq subword-separators \"_-\")
  (buffer-put 'subword-separators \"_-2\")")

(define-minor-mode subword-mode "Subword"
  "Move and kill by identifier parts in this buffer: lowerCase, HTTPServer and
letter/digit transitions, with literal separators from subword-separators.
A hyphen stays inside an identifier unless chosen as a separator.

Example:
  (subword-mode 1)")

(provide 'subword)




