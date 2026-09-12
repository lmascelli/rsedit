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

;; Windows. C-x 2 splits above/below, C-x 3 side by side, C-x 0 closes this
;; one, C-x 1 closes the others, C-x o cycles.
(define-key nil "C-x 2" 'split-window-below)
(define-key nil "C-x 3" 'split-window-right)
(define-key nil "C-x 0" 'delete-window)
(define-key nil "C-x 1" 'delete-other-windows)
(define-key nil "C-x o" 'other-window)

;; ... and once you are cycling, a bare `o' keeps going: C-x o o o walks
;; through the windows without the prefix each time. Any other key ends the
;; run and does its own job, so ignoring the offer costs nothing.
(define-repeat-key 'other-window "o")

;; Incremental search. C-s opens a prompt and the buffer jumps to the first
;; match of whatever has been typed so far, re-searching on every keystroke.
;; Inside the search, C-s goes to the next match and C-r turns around; Return
;; stops there, and Escape or C-g puts point back where it started. A search
;; that runs out says so, and repeating it then wraps around the buffer.
;;
;; C-M-s and C-M-r take a regular expression instead.
(define-key nil "C-s" 'isearch-forward)
(define-key nil "C-r" 'isearch-backward)
(define-key nil "C-M-s" 'isearch-forward-regexp)
(define-key nil "C-M-r" 'isearch-backward-regexp)

;; Whether searching ignores case. Emacs' name, Emacs' default -- and it is
;; also set from Rust, so searching behaves the same with no configuration
;; loaded at all.
(setq case-fold-search t)

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
