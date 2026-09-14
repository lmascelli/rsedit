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
        (setq *completion--index* 0)
        (setq *completion--on-choose* on-choose)
        (buffer-create completion-buffer-name 'completion-mode)
        (setq *completion--window*
              (display-buffer-at-bottom completion-buffer-name
                                        completion-window-height))
        (completion--draw)
        (completion--install-keys)))))

(defun completion--install-keys ()
  "Take over the keyboard until a candidate is chosen or the question dropped.

A modal map: every key it does not bind is swallowed. That is deliberate and it
is why `C-g' and Escape are both bound -- a map that answers nothing and
refuses everything is one the user cannot get out of, and nothing else can take
it down."
  (set-transient-keymap
   (list (cons "n" 'completion-next)
         (cons "p" 'completion-previous)
         (cons "<ret>" 'completion-choose)
         (cons "<esc>" 'completion-abandon)
         (cons "C-g" 'completion-abandon))
   "[n/p to move, RET to choose, ESC to cancel]"))

(defun completion-next ()
  "Select the next candidate, wrapping round at the end."
  (completion--move 1))

(defun completion-previous ()
  "Select the previous candidate, wrapping round at the beginning."
  (completion--move -1))

(defun completion--move (step)
  "Move the selection by STEP, and redraw if that crossed onto another page."
  (let* ((total (length *completion--items*))
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
  "Take the selected candidate and put the question away."
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

(setq *completion-read-function* 'completion--present)

(log "End of the completion.lisp")
