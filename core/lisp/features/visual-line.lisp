;;; visual-line --- word wrapping without changing the buffer's text.
;;; User manual: the `wrapping' page.
;;; Design: `doc/architettura-editor.typ, <righe-visive>`.
(require 'commands)
(require 'debug)

(define-minor-mode visual-line-mode "Wrap"
  "Wrap long lines at word boundaries in the current buffer, without inserting
newlines. With no argument toggle; a positive argument enables the mode,
zero or a negative argument disables it. The keys that move by lines -- the
arrows, C-p and C-n, Home, End, C-a and C-e -- follow visual rows.

Example:
  (visual-line-mode 1)")

;; No keymap of its own: the ordinary keys -- the arrows, C-p and C-n, Home,
;; End, C-a and C-e -- run the `move-' commands, which move by visual rows
;; where this mode is on. A mode that binds those keys for something else, as
;; a prompt's history or a list of candidates does, keeps them.

;; The rules of the roles: see the `modes' page, BUFFER ROLES. A text to read
;; wraps; a prompt and a list of candidates do not, and a view that is not a
;; document neither, unless the user's configuration says otherwise.
(minor-mode--distributed-policy 'visual-line-mode 'allow 'document-view)

(defun visual-line--turn-on ()
  "nil, after turning `visual-line-mode' on in the current buffer. The global
mode asks the buffer's role first: one that does not admit the mode is passed
over before this is called."
  (minor-mode-on 'visual-line-mode)
  nil)

(define-globalized-minor-mode
 global-visual-line-mode
 visual-line-mode
 visual-line--turn-on
 "Turn `visual-line-mode' on in every buffer, or off in all of them.
The buffers to come follow; with no argument, the other way from how it is.
A buffer whose role does not admit it, a prompt or a list of candidates, is
passed over.")

(provide 'visual-line)
