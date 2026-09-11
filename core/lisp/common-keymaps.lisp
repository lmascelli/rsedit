(define-key nil "<backspace>" 'delete-backward-char)

;; Movement. Emacs bindings where a single key sequence can express them --
;; C-a / C-e / M-f / M-b / M-< / M-> all fit; anything needing a prefix key
;; (C-x style) waits on multi-key sequences.
(define-key nil "C-a" 'beginning-of-line)
(define-key nil "C-e" 'end-of-line)
(define-key nil "C-f" 'forward-char)
(define-key nil "C-b" 'backward-char)
(define-key nil "C-n" 'next-line)
(define-key nil "C-p" 'previous-line)
(define-key nil "M-f" 'forward-word)
(define-key nil "M-b" 'backward-word)
(define-key nil "M-n" 'forward-paragraph)
(define-key nil "M-p" 'backward-paragraph)
(define-key nil "M-<" 'beginning-of-buffer)
(define-key nil "M->" 'end-of-buffer)
(define-key nil "M-g" 'goto-line)

;; Deletion, mirroring the movement bindings above. These discard the text
;; rather than saving it -- the kill ring is roadmap #20 -- but they carry the
;; Emacs names so the bindings will not change once it exists.
(define-key nil "C-d" 'delete-char)
(define-key nil "C-k" 'kill-line)
(define-key nil "M-d" 'kill-word)
(define-key nil "M-<backspace>" 'backward-kill-word)

;; Undo and redo. C-/ and C-_ are the same keystroke on most terminals -- the
;; terminal reports C-/ as C-_ -- so both are bound, and whichever one the
;; terminal sends arrives at the same command.
(define-key nil "C-/" 'undo)
(define-key nil "C-_" 'undo)
(define-key nil "M-_" 'redo)

;; The mark and the region. C-<space> sets the mark; moving point from there
;; grows the region, and any edit ends it.
(define-key nil "C-<space>" 'set-mark)

;; The kill ring. C-w and M-w act on the region; C-y puts the most recent kill
;; back and M-y walks further into the ring, but only straight after a yank.
(define-key nil "C-w" 'kill-region)
(define-key nil "M-w" 'kill-ring-save)
(define-key nil "C-y" 'yank)
(define-key nil "M-y" 'yank-pop)

;; The region is drawn with the `region' face, which ships bound to reverse
;; video -- no colour to clash with whatever scheme the terminal is set to. To
;; pick colours instead:
;;
;;   (set-face 'region nil "bright-black")         ; a conventional colour
;;   (set-face 'region "#f8f8f2" "#3a5fcd")        ; any colour you like
;;   (set-face 'region nil nil '("reverse"))       ; back to the default
;;
;; A colour is a request. The renderer draws it as asked where it can, and the
;; nearest thing it has where it cannot -- on a sixteen-colour terminal that is
;; a palette slot, which is your own configured colour. `list-colors' names the
;; conventional ones, and they are chosen so they land on the slot of the same
;; name when a terminal has to approximate.
;;
;; `list-faces' names everything that can be styled this way.

;; Prefix key sequences. A binding is written as its keys separated by spaces;
;; pressing the first shows it in the echo area and waits for the rest.
(define-key nil "C-x C-x" 'exchange-point-and-mark)
(define-key nil "C-x h" 'mark-whole-buffer)
(define-key nil "C-x C-f" 'find-file)

;; C-g abandons a half-typed key sequence or prefix argument, and ends the
;; region. It does not interrupt a running command -- that is roadmap #24.
(define-key nil "C-g" 'keyboard-quit)

;; Each window carries a status line on its bottom row. `mode-line-format' is
;; the string it shows, with Emacs' escapes:
;;
;;   %b buffer   %f file path   %m major mode
;;   %l line     %c column      %p where point is (All/Top/Bot/NN%)
;;   %* "**" when modified, "--" when not      %% a literal per cent
;;
;;   (setq mode-line-format " %* %b   %m   L%l C%c   %p ")   ; the default
;;
;; It is styled with the `mode-line' face, and `mode-line-inactive' for a
;; window that does not have focus -- see `set-face'.

(log "End of the common-keymaps.lisp")
