;;; overwrite --- typing over the text instead of pushing it along.
;;;
;;; The minor mode `overwrite-mode', on the Insert key: where it is on, a
;;; character typed takes the place of the one after point.
;;; User manual: the `editing' page.
;;; Design: `doc/architettura-editor.typ, <mod-overwrite>`.

(require 'commands)
(require 'debug)

(define-minor-mode overwrite-mode "Ovwrt"
  "Type over the text in the current buffer: a character typed takes the
place of the one after point, and is inserted at the end of a line. On, off,
or the other way with no argument.

Example:
  (overwrite-mode 1)")

(define-key nil "<insert>" 'overwrite-mode)

(log "overwrite loaded")
(provide 'overwrite)
