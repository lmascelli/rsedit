;;; auto-revert --- a buffer read again when its file changes on disk.
;;;
;;; The minor mode `auto-revert-mode', on in every buffer unless turned off:
;;; where it is on, the editor's file watcher compares the buffer with its
;;; file every few seconds.
;;; User manual: the `files' page.
;;; Design: `doc/architettura-editor.typ, <mod-auto-revert>`.

(require 'commands)
(require 'debug)

(define-minor-mode auto-revert-mode nil
  "Notice the current buffer's file changing on disk: without unsaved changes
the buffer is read again, and with them it is marked, so that saving it asks
first. On, off, or the other way with no argument; on in every buffer unless
turned off.

Example:
  (auto-revert-mode -1)")

(define-globalized-minor-mode global-auto-revert-mode auto-revert-mode nil
                              "Enable auto-revert-mode in every buffer")

(log "auto-revert loaded")
(provide 'auto-revert)
