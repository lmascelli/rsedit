;;; clipboard --- killed text also goes to the system clipboard.
;;;
;;; Two directions, two mechanisms, and the asymmetry is deliberate.
;;;
;;; Out: every kill and copy writes an OSC 52 escape to the terminal, which
;;; puts the text in the system clipboard. It costs nothing, needs no program
;;; installed, and -- unlike `pbcopy' or `wl-copy' -- it works over SSH,
;;; because the escape travels the same connection the text does and arrives
;;; wherever the terminal actually is.
;;;
;;; In: nothing here. The read direction of OSC 52 exists, but terminals
;;; disable it by default (it would let anything that can write to your
;;; terminal read your clipboard) and where it is allowed the answer arrives on
;;; stdin mixed in with your keystrokes. So the editor does not ask. Instead it
;;; enables *bracketed paste*, and when you press your terminal's own paste key
;;; the text arrives as one `insert-pasted-text' command -- one undo step, and
;;; no auto-pairing of brackets that were already balanced.
;;;
;;; Which means `C-y' is not the key that pastes from outside. `C-y' is the
;;; kill ring, which is a different and older thing: it remembers the last
;;; sixty kills, not the last one. Use the terminal's paste key for text from
;;; another program and `C-y' for text from this editor.

;; Read by `EditorState::kill' in Rust, which is the one funnel every kill and
;; copy command goes through -- so setting this covers `C-w', `M-w', `C-k',
;; `M-d', `C-M-k' and the rest, and there is no list of commands to keep up to
;; date.
;;
;; Unbound means off, which is why this line exists rather than a default
;; buried in the Rust: the test harness loads no Lisp, and every kill in every
;; test would otherwise queue a clipboard payload for a terminal that is not
;; there.
(setq clipboard-sync t)

(defun clipboard-sync-toggle ()
  "Turn the system clipboard on or off, reporting which it now is.

Worth having because the reasons to turn it off are occasional rather than
permanent -- editing something you would rather not leave lying in the
clipboard, or working on a machine whose clipboard is shared further than you
would like."
  (setq clipboard-sync (not clipboard-sync))
  (message (if clipboard-sync
               "Kills now go to the system clipboard"
               "Kills stay in the editor")))

(log "clipboard loaded")
