;;; clipboard --- killed text also goes to the system clipboard.
;;;
;;; Every kill and copy is sent to the terminal in an OSC 52 escape, which puts
;;; it in the system clipboard. Text comes in through the terminal's own paste,
;;; as one `insert-pasted-text'. User manual: the `killing' page.
;;;
;;; The editor never asks the terminal for the clipboard. Terminals disable
;;; that half of OSC 52 by default, since it would let anything that can write
;;; to the terminal read the clipboard, and where it is allowed the answer
;;; arrives on stdin mixed in with the keystrokes.

;; Read by `EditorState::kill' in Rust, the one funnel every kill and copy
;; goes through, so this covers them all. Unbound means off: the test harness
;; loads no Lisp, and its kills must not queue clipboard escapes for a
;; terminal that is not there.
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
