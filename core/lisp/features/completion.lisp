;;; completion --- the strip that shows every candidate at once.
;;;
;;; Tab at a prompt, and C-M-i in a buffer, show the candidates in a strip
;;; along the bottom of the frame, which typing narrows. Without this module
;;; Tab cycles through them in the prompt. User manual: the `completion' page.
;;;
;;; The editor only calls `*completion-read-function*', which this module
;;; sets, and the module knows nothing of where the candidates came from.
;;; The strip never takes the focus, so typing still reaches the prompt, and
;;; it is drawn a page at a time around the selection: the candidates are
;;; held here, and the buffer is only a picture of them.

(require 'debug)

(make-mode 'completion-mode)

(defconst completion-buffer-name "*Completions*"
  "The buffer the strip shows.")

(defvar completion-window-height 6
  "How many rows of candidates the strip shows.")

(defvar completion-columns 3
  "How many candidates the strip shows side by side.")

;; The module's own state: the candidates, the one selected, what to call with
;; the answer and the strip's window. Cleared when the strip closes, so that a
;; stale answer never reaches a question nobody asked.
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
;;
;; Set only for as long as the presenter is being called, so it is read as the
;; strip opens and never afterwards: by the next keystroke it is nil again.
(setq *completion-at-point-start* nil)

;; ---------------------------------------------------------------------------
;; Items
;; ---------------------------------------------------------------------------

(defun completion-value (item)
  "The text ITEM completes to: ITEM itself, a string, or the VALUE of a
(VALUE . DESCRIPTION) pair."
  (if (consp item) (car item) item))

(defun completion-description (item)
  "What ITEM says it is, the DESCRIPTION of a (VALUE . DESCRIPTION) pair, or
nil."
  (if (consp item) (cdr item) nil))

(defun completion--cell (item width)
  "ITEM in exactly WIDTH characters, cut short or padded, the last always a
space."
  ;; Exactly, padding included: where each candidate is in the buffer is
  ;; computed, not searched for -- see `completion--offset'. The last column
  ;; is kept blank so that a long candidate does not run into the next.
  (let* ((description (completion-description item))
         (text (if description
                   (concat (completion-value item) " " description)
                   (completion-value item)))
         (room (max 0 (- width 1)))
         (shown (if (< room (length text)) (substring text 0 room) text)))
    ;; Cut rather than wrapped: a candidate spilling onto the next row would
    ;; put two things in one cell.
    (concat shown (make-string (- width (length shown)) " "))))

;; ---------------------------------------------------------------------------
;; The page
;; ---------------------------------------------------------------------------

(defun completion--per-page ()
  "How many candidates one screenful of the strip holds."
  (* completion-window-height completion-columns))

(defun completion--page-start ()
  "The index of the first candidate on the page the selection is on."
  ;; `floor', because `/' divides exactly.
  (* (floor (/ *completion--index* (completion--per-page))) (completion--per-page)))

(defun completion--cell-width ()
  "How wide one column is, in characters."
  (max 1 (floor (/ frame-width completion-columns))))

(defun completion--offset (index)
  "Where candidate INDEX begins in the completions buffer."
  ;; Every row is `completion-columns' cells of one width and a newline.
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
  "Mark the selected candidate as the region, so that it is drawn highlighted."
  ;; The region rather than a marker character, which would push that cell
  ;; out of its column; it is drawn in a window without the focus too.
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
             (shown (min (max 0 (- width 1))
                         (length (completion-value item)))))
        (goto-char start)
        (set-mark)
        (goto-char (+ start shown))))))

;; ---------------------------------------------------------------------------
;; Opening, choosing, closing
;; ---------------------------------------------------------------------------

(defun completion--present (items on-choose)
  "Show ITEMS in the strip, and call ON-CHOOSE with the one chosen, if any.
What `*completion-read-function*' names."
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
  "Bind the keys that move and choose, and let every other key through."
  ;; A filter: typing reaches the prompt or the buffer, and narrows the list.
  ;; Nothing takes such a keymap down by itself, so Escape and C-g both close;
  ;; C-n and C-p rather than n and p, which have to reach the text.
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
  "The text the candidates are matched against."
  ;; What has been typed since the completion began in a buffer, the whole
  ;; prompt at a prompt, and nothing for a strip opened by neither.
  (cond
   (*completion--anchor*
    (if (>= (point) *completion--anchor*)
        (buffer-substring *completion--anchor* (point))
        ""))
   (t (let ((text (completion--minibuffer-text)))
        (if text text "")))))

(defun completion--narrow (text)
  "Make the candidates those for TEXT."
  ;; In a buffer they are filtered from the ones the strip opened with, since
  ;; typing does not change what could go there. A prompt is asked again: for
  ;; a path, a `/' typed means another directory's entries.
  (if *completion--anchor*
      (setq *completion--items* (fuzzy-filter text *completion--all-items*))
      (let ((fresh (if *minibuffer-on-change*
                       (funcall *minibuffer-on-change* text)
                       nil)))
        (setq *completion--all-items* fresh)
        (setq *completion--items* fresh))))

(defun completion--refresh ()
  "Narrow the strip again when what is typed has changed. On
`post-command-hook'."
  ;; Compared with the last text, because the strip's own C-n is a command
  ;; too, and narrowing after it would put the selection back at the top.
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

(defun completion-reconsider ()
  "Ask for the candidates again and redraw the strip, when one is open,
keeping the selection. What `completion-invalidate' reaches, through
`*completion-invalidated-function*'."
  ;; Unlike `completion--refresh', nothing was typed: the list grew.
  (if *completion--window*
      (let ((text (completion--current-text)))
        (completion--narrow text)
        (setq *completion--pattern* text)
        ;; A list that has grown can still have shrunk under the cursor -- a
        ;; rebuild, not an append -- so the selection is clamped rather than
        ;; assumed to be in range.
        (if (>= *completion--index* (length *completion--items*))
            (setq *completion--index* 0))
        (completion--draw))))

(defun completion-next ()
  "Select the next candidate, wrapping round at the end."
  (completion--move 1))

(defun completion-previous ()
  "Select the previous candidate, wrapping round at the beginning."
  (completion--move -1))

(defun completion--move (step)
  "Move the selection by STEP, and redraw when that turns the page."
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
  "Take the selected candidate and close the strip; with nothing matching, say
so and leave it open."
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
  "Close the strip without taking a candidate."
  (completion--close))

(defun completion--close ()
  "Take down the strip, its keymap and its state."
  ;; All three, every time, so that no way out leaves the keys captured.
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
;; The line below is what makes any of this run; setting the variable back to
;; nil brings back the cycling the editor does on its own.

(defconst completion-minibuffer-name "*Minibuffer*"
  "The buffer a prompt reads into, whose text the strip narrows by.")

(setq *completion-read-function* 'completion--present)

;; How a list built in the background tells the strip it has grown.
(setq *completion-invalidated-function* 'completion-reconsider)

;; How candidates are matched, in a buffer and as the strip narrows alike.
(setq *completion-filter-function* 'fuzzy-filter)

;; For every mode: a strip can be open in front of any buffer.
(add-hook nil "post-command-hook" 'completion--refresh)

;; M-x, matched as everything else is, so that M-x cap finds capitalize-word.
(defun completion-commands-fuzzy (pattern)
  "Command names matching PATTERN, best first."
  (fuzzy-filter pattern (all-commands)))

(setq *command-completion-function* 'completion-commands-fuzzy)

(log "End of the completion.lisp")
(provide 'completion)
