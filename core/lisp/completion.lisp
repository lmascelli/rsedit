;;; completion --- pick a candidate from a list, in a strip along the bottom.
;;;
;;; An optional module. Nothing in the editor requires it: without it, Tab in a
;;; prompt cycles through candidates one at a time, exactly as it always has.
;;; Loading this file sets `*completion-read-function*', and from then on Tab
;;; shows every candidate at once and lets you walk them with `n' and `p'.
;;; Unload it -- or set that variable back to nil -- and the cycling returns.
;;;
;;; # What the editor knows about this
;;;
;;; Nothing at all. `minibuffer-complete' asks whether
;;; `*completion-read-function*' is set; if it is, it hands over the candidate
;;; list and the symbol `minibuffer-choose-completion', and stops. It does not
;;; know what the presenter will do with them, and the presenter does not know
;;; the candidates came from a prompt. That is what makes this replaceable
;;; rather than merely configurable: a different picker is a different function
;;; in that variable, not a patch to the editor.
;;;
;;; # The strip, and why it is not a floating window
;;;
;;; `display-buffer-at-bottom' divides the whole frame, so the strip appears
;;; below everything and the windows above it give up the space. A float would
;;; paint over them instead -- which is right for a prompt, whose three lines
;;; are gone in a moment, and wrong for a list you read while comparing it
;;; against the text underneath.
;;;
;;; The strip never takes focus. If it did, its buffer would become the current
;;; one and the next keystroke would be typed into the list of suggestions
;;; rather than into the prompt the user is still filling in.
;;;
;;; # Why a page rather than a scrollbar
;;;
;;; Only the focused window scrolls to follow its cursor, and the strip is
;;; never focused. So instead of scrolling, the strip is *re-rendered* around
;;; the selection: it always shows the page the selection is on. The buffer is
;;; the viewport, which needs no scrolling machinery and cannot get out of step
;;; with what is selected.
;;;
;;; # The state
;;;
;;; Four variables, and they are the module's own: the candidates it was given,
;;; which one is selected, what to call with the answer, and which window it
;;; opened. Unlike `dired', the truth is not on screen and cannot be -- the
;;; candidate list is handed in and exists nowhere else -- so it is held here
;;; and the buffer is a rendering of it.

(make-mode 'completion-mode)

(defconst completion-buffer-name "*Completions*"
  "The buffer the strip shows.")

;; How many rows tall the strip is. All of them show candidates: a strip has no
;; status line, so this many rows means this many rows of completions.
(setq completion-window-height 6)

;; How many candidates sit side by side on a row.
(setq completion-columns 3)

;; The module's own state -- see the header. Cleared when the strip closes so
;; that a stale answer can never be delivered to a question nobody asked.
(setq *completion--items* nil)
(setq *completion--index* 0)
(setq *completion--on-choose* nil)
(setq *completion--window* nil)

;; The candidates as they were offered, before anything was typed to narrow
;; them. Kept because narrowing has to be redone from the whole list on every
;; keystroke: filtering the already-filtered list would make backspace unable
;; to widen the selection again.
(setq *completion--all-items* nil)

;; Where the text being completed begins, for an in-buffer completion, or nil
;; when the strip was opened by the minibuffer. This is what tells the refresh
;; below which of the two questions it is looking at -- the text between here
;; and point, or the minibuffer's contents.
(setq *completion--anchor* nil)

;; What had been typed the last time the strip was narrowed. The refresh runs
;; after *every* command, including the strip's own `C-n' -- so without
;; something to compare against, moving the selection would immediately reset
;; it to the first candidate and the arrow keys would do nothing at all.
(setq *completion--pattern* nil)

;; Set by `completion-at-point' just before it calls a presenter, and left nil
;; by the minibuffer. Given a value here because a presenter may also be called
;; directly -- by the minibuffer, or by a test -- and reading a variable that
;; nothing has set yet is an error rather than nil.
(setq *completion-at-point-start* nil)

;; ---------------------------------------------------------------------------
;; Items
;; ---------------------------------------------------------------------------

(defun completion-value (item)
  "The text ITEM completes to.

An item is either a string, which is its own value, or a (VALUE . DESCRIPTION)
pair. The bare string is what every completion source in the editor already
produces -- file names, buffer names, command names -- so none of them had to
change for this module to exist, and none of them has to change to gain a
description later."
  (if (consp item) (car item) item))

(defun completion-description (item)
  "What ITEM is, or nil if it did not say.

Shown after the value when there is room. Meant for what a language server
sends alongside a completion -- a type, a signature -- which is the reason the
pair exists at all rather than the list being plain strings."
  (if (consp item) (cdr item) nil))

(defun completion--cell (item width)
  "ITEM rendered into exactly WIDTH characters.

Exactly, including the padding, because the position of every candidate in the
buffer is worked out by arithmetic rather than by searching: a row is COLUMNS
cells of WIDTH, so candidate N begins at a place that can be computed. A cell
that were merely *at most* WIDTH would make every position after it a guess."
  (let* ((description (completion-description item))
         (text (if description
                   (concat (completion-value item) " " description)
                   (completion-value item)))
         (length (length text)))
    (if (< width length)
        ;; Truncated rather than wrapped: a candidate spilling onto the next
        ;; row would put two things in one cell and break the arithmetic above.
        (substring text 0 width)
        (concat text (make-string (- width length) " ")))))

;; ---------------------------------------------------------------------------
;; The page
;; ---------------------------------------------------------------------------

(defun completion--per-page ()
  "How many candidates one screenful of the strip holds."
  (* completion-window-height completion-columns))

(defun completion--page-start ()
  "The index of the first candidate on the page the selection is on.

`floor' because `/' here is division and not integer division: without it the
start of page 1 of 15 candidates is 6.6, and every position computed from it is
between two characters."
  (* (floor (/ *completion--index* (completion--per-page))) (completion--per-page)))

(defun completion--cell-width ()
  "How wide one column is, in characters."
  (max 1 (floor (/ frame-width completion-columns))))

(defun completion--offset (index)
  "Where candidate INDEX begins, as a position in the completions buffer.

Arithmetic, not a search: every row is COLUMNS cells of equal width followed by
a newline, so the position is a multiplication. This is the whole reason
`completion--cell' pads."
  (let* ((width (completion--cell-width))
         (relative (- index (completion--page-start)))
         (row (floor (/ relative completion-columns)))
         (column (% relative completion-columns)))
    (+ (* row (+ (* completion-columns width) 1)) (* column width))))

(defun completion--draw ()
  "Fill the completions buffer with the page the selection is on."
  ;; A lambda, because `with-current-buffer' here takes a function rather than a
  ;; body -- a primitive receives its arguments already evaluated, so a body
  ;; would have run in the wrong buffer before it ever arrived.
  (with-current-buffer completion-buffer-name
    (lambda ()
      (set-buffer-read-only nil)
      (clear-buffer)
      ;; Nothing matches what is typed. The strip stays -- the keystrokes went
      ;; to the buffer, so one backspace brings the list straight back, and a
      ;; strip that closed itself here would have to be reopened by hand.
      (if (null *completion--items*) (insert "No matches"))
      (let* ((width (completion--cell-width))
             (start (completion--page-start))
             (total (length *completion--items*))
             (index start))
        (dotimes (row completion-window-height)
          (dotimes (column completion-columns)
            ;; Past the end of the list, the cell is still written -- as
            ;; spaces. A short last row would make every position below it
            ;; wrong.
            (insert (if (< index total)
                        (completion--cell (nth index *completion--items*) width)
                        (make-string width " ")))
            (setq index (+ index 1)))
          (insert "\n")))
      (set-buffer-read-only t)))
  (completion--highlight))

(defun completion--highlight ()
  "Mark the selected candidate as the region, so it is drawn highlighted.

The region rather than a marker character: a `>' in front of the selection
would shift that cell's text by one and make it the only cell not aligned with
its column. The region is also what the renderer already knows how to draw, in
every window showing the buffer -- including one that is not focused, which
this one never is."
  (if (null *completion--items*)
      nil
      (completion--highlight-selected)))

(defun completion--highlight-selected ()
  "Mark the selected candidate, there being one to mark."
  (with-current-buffer completion-buffer-name
    (lambda ()
      (let* ((item (nth *completion--index* *completion--items*))
             (start (completion--offset *completion--index*))
             (width (completion--cell-width))
             (shown (min width (length (completion-value item)))))
        (goto-char start)
        (set-mark)
        (goto-char (+ start shown))))))

;; ---------------------------------------------------------------------------
;; Opening, choosing, closing
;; ---------------------------------------------------------------------------

(defun completion--present (items on-choose)
  "Show ITEMS in the strip and let the user pick one.

This is what `*completion-read-function*' names, and the whole of what a
replacement has to provide: take a list of items, and eventually call ON-CHOOSE
with the chosen value -- or do not call it at all, if the user declined."
  (cond
   ((null items) (message "No completions"))
   ;; One candidate is not a choice. Taking it saves a keystroke and, more to
   ;; the point, opening a window to ask a question with one answer reads as
   ;; the editor not knowing what it is doing.
   ((= 1 (length items)) (funcall on-choose (completion-value (car items))))
   (t (progn
        (setq *completion--items* items)
        ;; Kept unnarrowed, so that backspace can widen the list again. Only
        ;; ever filtered *from* here, never in place.
        (setq *completion--all-items* items)
        (setq *completion--index* 0)
        (setq *completion--on-choose* on-choose)
        ;; Which question this is. `*completion-at-point-start*' is set by
        ;; `completion-at-point' before it calls a presenter and left nil by
        ;; the minibuffer, so it doubles as the answer to "where does the text
        ;; I am matching against come from".
        (setq *completion--anchor* *completion-at-point-start*)
        (setq *completion--pattern* (completion--current-text))
        (buffer-create completion-buffer-name 'completion-mode)
        (setq *completion--window*
              (display-buffer-at-bottom completion-buffer-name
                                        completion-window-height))
        (completion--draw)
        (completion--install-keys)))))

(defun completion--install-keys ()
  "Bind the keys that move and choose, and let everything else through.

A *filter* map rather than a modal one, which is the whole of this module's
behaviour as far as a user notices it. The strip used to swallow every key it
did not bind, so a list of forty candidates could only be walked through --
there was no way to say which one you wanted except by pressing `n' until you
reached it. Now typing goes to the buffer or the minibuffer underneath, and
what you type narrows the list.

Passing keys on means nothing dismisses the map on its own, so the ways out
matter more than they did: Escape and `C-g' are both bound, and both close.

`C-n' and `C-p' rather than bare `n' and `p', because bare letters are exactly
what has to reach the buffer now."
  (set-transient-keymap
   (list (cons "C-n" 'completion-next)
         (cons "C-p" 'completion-previous)
         (cons "<down>" 'completion-next)
         (cons "<up>" 'completion-previous)
         (cons "<ret>" 'completion-choose)
         ;; Tab is usually what opened the strip, so pressing it again taking
         ;; the selection is the natural second tap.
         (cons "<tab>" 'completion-choose)
         (cons "<esc>" 'completion-abandon)
         (cons "C-g" 'completion-abandon))
   "[type to narrow, C-n/C-p to move, RET to choose, ESC to cancel]"
   ;; Pass unbound keys through, and stay up.
   t))

;; ---------------------------------------------------------------------------
;; Narrowing as you type
;; ---------------------------------------------------------------------------

(defun completion--minibuffer-text ()
  "What is in the minibuffer, or nil if there is no minibuffer open."
  ;; The minibuffer may already have been taken down -- a command that closed
  ;; it can run before this hook does -- and `with-current-buffer' signals
  ;; rather than returning nil for a buffer that is not there.
  (if (member completion-minibuffer-name (all-buffer-names))
      (with-current-buffer completion-minibuffer-name (lambda () (buffer-string)))
      nil))

(defun completion--current-text ()
  "The text the candidates should be matched against.

The two callers put it in different places, and this is the only function that
has to know: an in-buffer completion is matching against what has been typed
since the completion began, and a minibuffer prompt against the whole of what
is in the minibuffer.

A strip opened by neither -- a module calling the presenter directly -- has no
text to match against at all, and answers the empty string. It shows what it
was given and does not narrow, which is what it did before any of this."
  (cond
   (*completion--anchor*
    (if (>= (point) *completion--anchor*)
        (buffer-substring *completion--anchor* (point))
        ""))
   (t (let ((text (completion--minibuffer-text)))
        (if text text "")))))

(defun completion--narrow (text)
  "Recompute the candidates for TEXT.

The two callers are narrowed differently, and deliberately so.

An in-buffer completion is filtered from the candidates it was opened with:
they were computed once by asking the mode what could go at that position, and
typing does not change what could go there -- only which of them you meant.

A minibuffer prompt is *re-asked* instead. Its candidates come from
`*minibuffer-on-change*' called with the whole input, and for `find-file' that
input is a path: typing a `/' means the answer is now a different directory's
entries, which no amount of filtering the old list could produce."
  (if *completion--anchor*
      (setq *completion--items* (fuzzy-filter text *completion--all-items*))
      (let ((fresh (if *minibuffer-on-change*
                       (funcall *minibuffer-on-change* text)
                       nil)))
        (setq *completion--all-items* fresh)
        (setq *completion--items* fresh))))

(defun completion--refresh ()
  "Re-narrow the strip if what was typed has changed.

Registered on `post-command-hook' for every mode, so it runs after anything the
user does while the strip is open -- including the strip's own commands. Which
is why it compares against the last pattern rather than simply re-narrowing:
`C-n' is a command too, and re-narrowing after it would reset the selection to
the first candidate and make the key appear to do nothing."
  (if *completion--window*
      (cond
       ;; Backspaced past where the completion began. There is no longer a word
       ;; being completed, so there is nothing to be completing -- and the span
       ;; between the anchor and point has turned inside out, which nothing
       ;; below could read.
       ((and *completion--anchor* (< (point) *completion--anchor*))
        (completion--close))
       ;; A strip nothing is typing into -- opened by a module calling the
       ;; presenter directly rather than by `completion-at-point' or the
       ;; minibuffer. There is no text to narrow by, so it is left as it was
       ;; rather than being closed: closing it here would take down every strip
       ;; that did not come from one of the two built-in callers, including the
       ;; one a `completion-choose' callback had just opened.
       ((and (null *completion--anchor*) (null (completion--minibuffer-text)))
        nil)
       (t (let ((text (completion--current-text)))
            (if (not (equal text *completion--pattern*))
                (progn
                  (setq *completion--pattern* text)
                  (completion--narrow text)
                  (setq *completion--index* 0)
                  (completion--draw))))))))

(defun completion-next ()
  "Select the next candidate, wrapping round at the end."
  (completion--move 1))

(defun completion-previous ()
  "Select the previous candidate, wrapping round at the beginning."
  (completion--move -1))

(defun completion--move (step)
  "Move the selection by STEP, and redraw if that crossed onto another page."
  (let* ((total (length *completion--items*))
         ;; Nothing to move through, and `mod' by zero is an error rather than
         ;; a no-op. Reachable by pressing `C-n' while "No matches" is up.
         (total (if (= total 0) 1 total))
         ;; `+ total' before the modulus so that stepping back from the first
         ;; candidate reaches the last rather than a negative index.
         ;;
         ;; It cannot be observed today: this `mod' already gives 2 for
         ;; (% -1 3), so `p' on the first candidate wraps either way. It stays
         ;; because that is a property of `mod', documented in the Rust source
         ;; as *not* following Elisp's rule in every case -- and the day it is
         ;; made to, this line should not be what breaks.
         (next (% (+ *completion--index* step total) total))
         (page (completion--page-start)))
    (setq *completion--index* next)
    (if (= page (completion--page-start))
        (completion--highlight)
        (completion--draw))))

(defun completion-choose ()
  "Take the selected candidate and put the question away.

With nothing matching there is nothing to take, and the strip stays up: what
you have typed is still in the buffer, and one backspace brings the list back."
  (if (null *completion--items*)
      (message "No matches")
      (completion--choose-selected)))

(defun completion--choose-selected ()
  "Hand the selected candidate to whoever asked for one."
  (let ((chosen (completion-value (nth *completion--index* *completion--items*)))
        (on-choose *completion--on-choose*))
    ;; Closed before the callback runs, not after: the callback may open a
    ;; prompt, or another completion, and it must not find this one's window
    ;; still on screen or its state still set.
    (completion--close)
    (funcall on-choose chosen)))

(defun completion-abandon ()
  "Put the question away without answering it."
  (completion--close))

(defun completion--close ()
  "Take down the strip, the keymap and the state, in that order.

All three, every time, through one function -- so that there is no way to end
this that leaves the keyboard captured by a window that is no longer there."
  (if *completion--window* (delete-window *completion--window*))
  (setq *completion--window* nil)
  (setq *completion--items* nil)
  (setq *completion--all-items* nil)
  (setq *completion--anchor* nil)
  (setq *completion--pattern* nil)
  (setq *completion--index* 0)
  (setq *completion--on-choose* nil)
  (clear-transient-keymap))

;; ---------------------------------------------------------------------------
;; Taking over
;; ---------------------------------------------------------------------------
;;
;; The one line that makes any of this run. Everything above is reachable
;; without it -- `completion--present' can be called directly -- and setting
;; this back to nil restores the cycling the editor does on its own.

(defconst completion-minibuffer-name "*Minibuffer*"
  "The buffer a minibuffer prompt reads into.

Named here because `completion--current-text' has to read what is in it, and a
literal repeated in two modules is one rename away from being wrong in one of
them.")

(setq *completion-read-function* 'completion--present)

;; How candidates are narrowed, everywhere at once. `completion-at-point'
;; consults this before presenting, and the strip uses the same function as you
;; type, so the two agree by construction rather than by being kept in step.
(setq *completion-filter-function* 'fuzzy-filter)

;; The refresh, for every mode. Registered globally -- `add-hook' with nil --
;; because what the buffer's major mode happens to be has nothing to do with
;; whether a completion strip is open in front of it, and registering this in
;; each mode separately would work until somebody defined a mode afterwards.
(add-hook nil "post-command-hook" 'completion--refresh)


;; M-x, matched the same way everything else is.
;;
;; `command-completions' narrows by prefix, which is the right default with no
;; module loaded and the wrong one once fuzzy matching exists: `M-x cap' should
;; find `completion-at-point', and by prefix it finds nothing. The seam for
;; this already existed -- M-x asks `*command-completion-function*' and only
;; falls back to the prefix version when it is unset.
(defun completion-commands-fuzzy (pattern)
  "Command names matching PATTERN, best first."
  (fuzzy-filter pattern (all-commands)))

(setq *command-completion-function* 'completion-commands-fuzzy)

(log "End of the completion.lisp")
