;;; completion --- the strip that shows every candidate at once.
;;;
;;; Tab at a prompt, and C-M-i in a buffer, show the candidates in a strip
;;; along the bottom of the frame, which typing narrows. Without this module
;;; Tab cycles through them in the prompt. User manual: the `completion' page.
;;; Design: `doc/architettura-editor.typ, <mod-completion>`.

(require 'debug)

(make-mode 'completion-mode)

(defconst completion-buffer-name "*Completions*"
  "The buffer the strip shows.")

(defvar completion-window-height 6
  "How many rows of candidates the strip shows.")

(defvar completion-columns 3
  "How many candidates the strip shows side by side.")

;; The candidates, the one selected, what to call with the answer and the
;; strip's window. Cleared when the strip closes.
(setq *completion--items* nil)
(setq *completion--index* 0)
(setq *completion--on-choose* nil)
(setq *completion--window* nil)

;; The candidates as offered, before any narrowing, so that backspace can
;; widen the list again.
(setq *completion--all-items* nil)

;; Where the text being completed begins in a buffer, or nil for a prompt:
;; which of the two texts the strip narrows by.
(setq *completion--anchor* nil)

;; What was typed when the strip was last narrowed: the refresh runs after the
;; strip's own C-n too, and must not reset the selection then.
(setq *completion--pattern* nil)

;; Set by `completion-at-point' while it calls a presenter, nil otherwise;
;; bound here, since reading an unbound variable is an error.
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
  ;; Exactly WIDTH: `completion--offset' computes where each candidate is.
  (let* ((description (completion-description item))
         (text (if description
                   (concat (completion-value item) " " description)
                   (completion-value item)))
         (room (max 0 (- width 1)))
         (shown (if (< room (length text)) (substring text 0 room) text)))
    ;; Cut, not wrapped, so that a cell holds one candidate.
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
  ;; A lambda: this `with-current-buffer' is a primitive, which takes a
  ;; function, since a body would be evaluated before it is called.
  (with-current-buffer completion-buffer-name
    (lambda ()
      (set-buffer-read-only nil)
      (clear-buffer)
      ;; Nothing matches: the strip stays, so that a backspace brings the list
      ;; back.
      (if (null *completion--items*) (insert "No matches"))
      (let* ((width (completion--cell-width))
             (start (completion--page-start))
             (total (length *completion--items*))
             (index start))
        (dotimes (row completion-window-height)
          (dotimes (column completion-columns)
            ;; Past the end of the list, spaces: the offsets count every cell.
            (insert (if (< index total)
                        (completion--cell (nth index *completion--items*) width)
                        (make-string width " ")))
            (setq index (+ index 1)))
          (insert "\n")))
      (set-buffer-read-only t)))
  (completion--highlight))

(defun completion--highlight ()
  "Mark the selected candidate as the region, so that it is drawn highlighted."
  ;; The region, not a marker character, which would shift the cell; it is
  ;; drawn in a window without the focus too.
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
   ;; One candidate is not a choice: taken, without a strip.
   ((= 1 (length items)) (funcall on-choose (completion-value (car items))))
   (t (progn
        (setq *completion--items* items)
        ;; Kept unnarrowed: filtered from, never in place.
        (setq *completion--all-items* items)
        (setq *completion--index* 0)
        (setq *completion--on-choose* on-choose)
        ;; In a buffer or at a prompt: see `*completion-at-point-start*'.
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
  ;; A filter: typing reaches the text and narrows the list. C-n and C-p, since
  ;; n and p are typed; Escape and C-g close, since nothing else would.
  (set-transient-keymap
   (list (cons "C-n" 'completion-next)
         (cons "C-p" 'completion-previous)
         (cons "<down>" 'completion-next)
         (cons "<up>" 'completion-previous)
         (cons "<ret>" 'completion-choose)
         ;; Tab opened the strip, and a second Tab takes the selection.
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
  ;; The prompt may be closed already, and `with-current-buffer' signals for a
  ;; buffer that is not there.
  (if (member completion-minibuffer-name (all-buffer-names))
      (with-current-buffer completion-minibuffer-name (lambda () (buffer-string)))
      nil))

(defun completion--current-text ()
  "The text the candidates are matched against."
  (cond
   (*completion--anchor*
    (if (>= (point) *completion--anchor*)
        (buffer-substring *completion--anchor* (point))
        ""))
   (t (let ((text (completion--minibuffer-text)))
        (if text text "")))))

(defun completion--narrow (text)
  "Make the candidates those for TEXT."
  ;; In a buffer, filtered from the first list; a prompt is asked again, since
  ;; a `/' typed in a path means another directory's entries.
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
  (if *completion--window*
      (cond
       ;; Backspaced past where the completion began: nothing is being
       ;; completed.
       ((and *completion--anchor* (< (point) *completion--anchor*))
        (completion--close))
       ;; A strip opened by calling the presenter directly has no text to
       ;; narrow by: left as it is, not closed.
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
        ;; A rebuild, not an append: the selection may be out of range.
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
         ;; C-n under "No matches": a modulus by zero is an error.
         (total (if (= total 0) 1 total))
         ;; `+ total', so that stepping back from the first candidate wraps
         ;; whatever sign `%' gives.
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
    ;; Closed before the callback, which may open another strip.
    (completion--close)
    (funcall on-choose chosen)))

(defun completion-abandon ()
  "Close the strip without taking a candidate."
  (completion--close))

(defun completion--close ()
  "Take down the strip, its keymap and its state."
  ;; Window, state and keymap, so that no way out leaves the keys captured.
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
;; `*completion-read-function*' set back to nil brings back the cycling the
;; editor does on its own.

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
