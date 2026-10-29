;;; clipboard --- killed text also goes to the system clipboard.
;;;
;;; Every kill and copy is handed to the front end, which puts it in the system
;;; clipboard; text comes back in as a paste. User manual: the `killing' page.
;;; Design: `doc/architettura-editor.typ, <mod-clipboard>`.

(require 'debug)

;; Read by `EditorState::kill', which every kill and copy goes through.
;; Unbound means off, as in the tests, which load no Lisp.
(defvar clipboard-sync t
  "Whether killed and copied text also goes to the system clipboard.")

;; A `defun' and a `register-command' rather than `defcommand', so that this
;; file loads without commands.lisp before it.
(defun clipboard-sync-toggle ()
  "Stop kills reaching the system clipboard, or let them again, and say which."
  (setq clipboard-sync (not clipboard-sync))
  (message (if clipboard-sync
               "Kills now go to the system clipboard"
               "Kills stay in the editor")))
(register-command "clipboard-sync-toggle" nil)

(log "clipboard loaded")
(provide 'clipboard)
