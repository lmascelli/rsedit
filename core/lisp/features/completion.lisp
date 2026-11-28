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

;; The window focused when the strip opened, which gets the focus back when
;; the strip closes with the focus in it, given by C-x o.
(setq *completion--origin* nil)

;; The buffer current then, and how many windows the frame held with the strip
;; open: another buffer, or a window closed, and the strip closes.
(setq *completion--origin-buffer* nil)
(setq *completion--window-count* 0)

;; Whether the strip's keys are installed: a prompt's strip stays up while the
;; focus is in another window, as in Emacs, but without them.
(setq *completion--keys* nil)

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

;; What a choice in a buffer replaces, (START END LENGTH BUFFER ...), set by
;; `completion-at-point' too; bound here for the same reason.
(setq *completion-at-point-region* nil)

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
      ;; Written by the strip, not by you: nothing to save.
      (set-buffer-modified-p nil)
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
        (buffer-create completion-buffer-name :mode 'completion-mode :role 'completion)
        (setq *completion--origin* (selected-window))
        (setq *completion--origin-buffer* (current-buffer))
        (setq *completion--window*
              (display-buffer-at-bottom completion-buffer-name
                                        completion-window-height))
        (setq *completion--window-count* (count-windows))
        (completion--draw)
        (completion--install-keys)))))

(defun completion--install-keys ()
  "Bind the keys that move and choose, and let every other key through."
  ;; A filter: typing reaches the text and narrows the list. C-n and C-p, since
  ;; n and p are typed; Escape and C-g close, since nothing else would.
  (setq *completion--keys* t)
  (set-transient-keymap
   (list (cons "C-n" 'completion-next)
         (cons "C-p" 'completion-previous)
         (cons "<down>" 'completion-next)
         (cons "<up>" 'completion-previous)
         ;; The strip draws one page at a time, so the scroll keys turn it,
         ;; from the prompt and from the strip itself alike.
         (cons "C-v" 'completion-next-page)
         (cons "M-v" 'completion-previous-page)
         (cons "<next>" 'completion-next-page)
         (cons "<prior>" 'completion-previous-page)
         (cons "<ret>" 'completion-choose)
         ;; Tab opened the strip, and a second Tab takes the selection.
         (cons "<tab>" 'completion-choose)
         (cons "<esc>" 'completion-abandon)
         (cons "C-g" 'completion-abandon))
   "[type to narrow, C-n/C-p to move, C-v/M-v to page, RET to choose, ESC to cancel]"
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
   ;; The focus in the strip: nothing is being typed, and its point is not
   ;; where the text is.
   ((equal (current-buffer) completion-buffer-name)
    (if *completion--pattern* *completion--pattern* ""))
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
  "Narrow the strip again when what is typed has changed, and close it when
the focus or point has left what it completes. On `post-command-hook'."
  (if *completion--window*
      (cond
       ;; A window closed, the strip's own by C-x 0 among them.
       ((not (= (count-windows) *completion--window-count*))
        (completion--close))
       ;; The focus in the strip, moving the selection: nothing was typed.
       ((equal (current-buffer) completion-buffer-name) (completion--keys-on))
       ;; Another window, or another buffer in this one: a prompt's strip
       ;; waits for the prompt without its keys, any other closes.
       ((not (and (equal (selected-window) *completion--origin*)
                  (equal (current-buffer) *completion--origin-buffer*)))
        (if (completion--prompt-open-p)
            (completion--keys-off)
            (completion--close)))
       ;; Backspaced past where the completion began, or moved on past the end
       ;; of the text: nothing is being completed.
       ((and *completion--anchor*
             (or (< (point) *completion--anchor*) (completion--past-end-p)))
        (completion--close))
       ;; A strip opened by calling the presenter directly has no text to
       ;; narrow by: left as it is, not closed.
       ((and (null *completion--anchor*) (null (completion--minibuffer-text)))
        nil)
       (t (progn
            (completion--keys-on)
            (let ((text (completion--current-text)))
              (if (not (equal text *completion--pattern*))
                  (progn
                    (setq *completion--pattern* text)
                    (completion--narrow text)
                    (setq *completion--index* 0)
                    (completion--draw)))))))))

(defun completion--prompt-open-p ()
  "Whether the strip was opened at a prompt that is still open."
  (and (equal *completion--origin-buffer* completion-minibuffer-name)
       (member completion-minibuffer-name (all-buffer-names))))

(defun completion--keys-off ()
  "Take the strip's keys away, the strip staying up."
  (if *completion--keys*
      (progn
        (setq *completion--keys* nil)
        (clear-transient-keymap))))

(defun completion--keys-on ()
  "Give the strip its keys back, if they were taken away."
  (if (not *completion--keys*) (completion--install-keys)))

(defun completion--past-end-p ()
  "Whether point is past the end of the text a choice would replace, which
grows with what is typed into it."
  (let ((region *completion-at-point-region*))
    (and region
         (> (point) (+ (nth 1 region) (- (point-max) (nth 2 region)))))))

(defun completion-reconsider (&optional items)
  "Ask for the candidates again and redraw the strip, when one is open,
keeping the selection. What `completion-invalidate' reaches, through
`*completion-invalidated-function*', and `completion-at-point' too, with
ITEMS, the candidates it has found so far, while it still asks its sources."
  ;; Unlike `completion--refresh', nothing was typed: the list grew.
  (if *completion--window*
      (let ((text (completion--current-text))
            (selected (if *completion--items*
                          (completion-value
                           (nth *completion--index* *completion--items*))
                          nil)))
        ;; ITEMS are a buffer's: a prompt's strip asks its own again.
        (if (and items *completion--anchor*)
            (setq *completion--all-items* items))
        (completion--narrow text)
        (setq *completion--pattern* text)
        ;; A rebuild, not an append: the one selected may have moved, or gone,
        ;; and then the first is.
        (setq *completion--index*
              (completion--position selected *completion--items*))
        (completion--draw))))

(defun completion--position (value items)
  "Where the candidate VALUE is in ITEMS, or 0 when it is not there."
  (let ((index 0)
        (found nil))
    (dolist (item items)
      (if (and (null found) (equal (completion-value item) value))
          (setq found index))
      (setq index (+ index 1)))
    (if found found 0)))

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

(defun completion-next-page ()
  "Show the next page of candidates, selecting its first, wrapping round at
the end."
  (completion--move-page 1))

(defun completion-previous-page ()
  "Show the previous page of candidates, selecting its first, wrapping round
at the beginning."
  (completion--move-page -1))

(defun completion--move-page (step)
  "Turn the strip by STEP pages, and select the first candidate on the page."
  (let* ((per-page (completion--per-page))
         ;; "No matches" is one page, as one page of candidates is.
         (pages (max 1 (floor (/ (+ (length *completion--items*) per-page -1)
                                 per-page))))
         (page (floor (/ *completion--index* per-page)))
         ;; `+ pages', as in `completion--move'.
         (next (% (+ page step pages) pages)))
    (setq *completion--index* (* next per-page))
    (completion--draw)))

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
  "Take down the strip, its buffer, its keymap and its state."
  ;; Window, state and keymap, so that no way out leaves the keys captured.
  (let ((focused (and *completion--window*
                      (equal (selected-window) *completion--window*))))
    ;; Its window may be gone already, closed by C-x 0.
    (if *completion--window*
        (condition-case nil (delete-window *completion--window*) (error nil)))
    ;; Closed from inside, the focus goes back where the completion began,
    ;; not to whichever window is first.
    (if (and focused *completion--origin*) (select-window *completion--origin*)))
  (if (member completion-buffer-name (all-buffer-names))
      (kill-buffer completion-buffer-name))
  (setq *completion--window* nil)
  (setq *completion--origin* nil)
  (setq *completion--origin-buffer* nil)
  (setq *completion--window-count* 0)
  (setq *completion--items* nil)
  (setq *completion--all-items* nil)
  (setq *completion--anchor* nil)
  (setq *completion--pattern* nil)
  (setq *completion--index* 0)
  (setq *completion--on-choose* nil)
  ;; Not when taken away already: the map there now is another's.
  (if *completion--keys* (clear-transient-keymap))
  (setq *completion--keys* nil))

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
