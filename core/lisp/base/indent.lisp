;;; indent --- what Tab and Return do.
;;;
;;; Tab indents the region, or the line, at the column the mode's
;;; `indent-function' returns, or like the line above in a mode without one;
;;; Return indents the line it opens. User manual: the `indentation' page.
;;;
;;; A rule returns a column rather than doing the work. A mode can only be
;;; wrong about a number; `indent-region' can call it a hundred times without
;;; a hundred chances to mangle a line; and where point ends up afterwards is
;;; written once, here and in `indent-line-to', instead of once per language.

(require 'commands)

(defvar tab-width 4
  "How many columns a Tab indents by, in a mode that does not say: see
`indent-width'.")

(defvar tab-always-indent nil
  "What Tab does on a line that is already indented: nil inserts
`indent-width' spaces, t does nothing, and the symbol complete calls
`completion-at-point'.")

(defun indent-width ()
  "Return how many columns a Tab indents by here: the mode's own, set with a
tab-width property on the mode, or `tab-width'."
  (or (get (major-mode) 'tab-width) tab-width))

(defun indent-line ()
  "Indent the line point is on at the column its mode's `indent-function'
returns, or like the line above. Return t if the line moved, nil if it was
already there."
  ;; Point is put back before the line is touched. An indent function has no
  ;; way to look at the buffer except by moving point -- `lisp-indent-line'
  ;; walks to the enclosing delimiter and back -- so by the time it answers,
  ;; point is wherever it finished, and `indent-line-to' would indent that line
  ;; instead of this one. Saving the offset is safe because nothing has been
  ;; edited yet: the rule only reads.
  (let* ((here (point))
         (indenter (get (major-mode) 'indent-function))
         (column (if indenter (funcall indenter) (previous-indentation))))
    (goto-char here)
    (indent-line-to column)))

;; ---------------------------------------------------------------------------
;; Regions
;; ---------------------------------------------------------------------------
;;
;; Everything below works on a region by line *number*, not by position:
;; indenting a line changes every offset below it, so a loop that remembered
;; where the region ended would be indenting the wrong text by the third line.
;;
;; And everything puts the region back when it is done, because it cannot be
;; preserved -- every edit deactivates the mark at the chokepoint, so it is
;; gone by the second line. Putting it back is what makes pressing Tab twice
;; work on the same block.

(defun indent--region-lines ()
  "The first and last line numbers the region covers, as (FROM TO)."
  ;; A region that ends at the very start of a line does not cover that line:
  ;; selecting whole lines from the left margin ends there, on the line after
  ;; the last one meant.
  ;;
  ;; Both ends first, before anything moves. The region is the span between
  ;; mark and point, so `(goto-char (region-beginning))' collapses it -- and
  ;; `region-end' then answers with the mark rather than the end that was
  ;; there a moment ago.
  (let* ((start (region-beginning))
         (end (region-end))
         (from (progn (goto-char start) (line-number-at-point)))
         (to (progn
               (goto-char end)
               (if (and (> end start) (= (current-column) 0))
                   (- (line-number-at-point) 1)
                   (line-number-at-point)))))
    (list from (max from to))))

(defun indent--reselect (from to)
  "Make lines FROM to TO the region again, whole."
  (goto-line from)
  (beginning-of-line)
  (set-mark)
  (goto-line to)
  (end-of-line))

(defun indent--text-column ()
  "The column this line's text starts at, or nil if the line is blank."
  (back-to-indentation)
  (let ((column (current-column))
        (here (point)))
    (end-of-line)
    (if (= here (point)) nil column)))

(defun indent-rigidly-lines (from to amount)
  "Shift lines FROM to TO by AMOUNT columns, keeping their shape.

A negative AMOUNT shifts them left, and a line with less indentation than that
goes to column 0. Blank lines are left alone."
  (dotimes (n (+ 1 (- to from)))
    (goto-line (+ from n))
    (let ((column (indent--text-column)))
      (when column
        (indent-line-to (max 0 (+ column amount)))))))

(defvar indent-region-style 'auto
  "What Tab does to a region, one of these symbols:

  reindent  put every line where the mode's `indent-function' says
  shift     move the block right by one `indent-width', keeping its shape
  auto      reindent in a mode that has an `indent-function', shift in one
            that does not

A mode can have its own: (put 'rust-mode 'indent-region-style 'shift).")

(defun indent--region-style ()
  "Whether Tab reindents or shifts the region here: `reindent' or `shift'."
  (let ((style (or (get (major-mode) 'indent-region-style) indent-region-style)))
    (cond
     ((eq style 'reindent) 'reindent)
     ((eq style 'shift) 'shift)
     ;; `auto', and anything not understood -- a typo should get the sensible
     ;; behaviour rather than an error on every Tab.
     ((get (major-mode) 'indent-function) 'reindent)
     (t 'shift))))

(defun indent-region ()
  "Indent every line the region touches as `indent-region-style' says, and
select the same lines again."
  (let* ((lines (indent--region-lines))
         (from (nth 0 lines))
         (to (nth 1 lines)))
    (if (eq (indent--region-style) 'reindent)
        (dotimes (n (+ 1 (- to from)))
          (goto-line (+ from n))
          (indent-line))
        (indent-rigidly-lines from to (indent-width)))
    (indent--reselect from to)))

(defun indent--shift (direction)
  "Shift the region, or the line, one `indent-width' right (1) or left (-1)."
  (let ((amount (* direction (indent-width))))
    (if (use-region-p)
        (let* ((lines (indent--region-lines))
               (from (nth 0 lines))
               (to (nth 1 lines)))
          (indent-rigidly-lines from to amount)
          (indent--reselect from to))
        ;; Point is put back before the line moves, so that `indent-line-to''s
        ;; own rule -- point keeps the character it was on -- applies to where
        ;; the user was rather than to wherever measuring the line left it. A
        ;; blank line still shifts here, unlike in a region: it is the line
        ;; the user asked about, not one swept up with the others.
        (let* ((here (point))
               (column (or (indent--text-column) (current-indentation))))
          (goto-char here)
          (indent-line-to (max 0 (+ column amount)))))))

(defcommand indent-rigidly-right () nil
  "Shift the region, or the line, right by one `indent-width', keeping its
shape."
  (indent--shift 1))

(defcommand indent-rigidly-left () nil
  "Shift the region, or the line, left by one `indent-width', keeping its
shape. A line already at the margin stays there."
  (indent--shift -1))

(defcommand indent-for-tab-command () nil
  "Indent the region, or the line; on a line that is already indented, do
what `tab-always-indent' says."
  (if (use-region-p)
      (indent-region)
      (if (indent-line)
          t
          (cond
           ((eq tab-always-indent 'complete) (completion-at-point))
           (tab-always-indent nil)
           (t (insert (make-string (indent-width) " ")))))))

;; ---------------------------------------------------------------------------
;; Indenting a Lisp
;; ---------------------------------------------------------------------------
;;
;; The rule risp-mode uses. It reads the structure rather than the text:
;; `syntax-ppss' says how deep point is and where its enclosing list opened,
;; so it needs no regular expressions and no list of keywords, and works in
;; any mode whose syntax table pairs parentheses.

(defun lisp-indent-line ()
  "Return the column the line point is on should start at, in a Lisp."
  (let ((state (syntax-ppss)))
    (cond
     ;; Inside a string the line's leading whitespace is content, not layout.
     ;; Reporting the indentation it already has is how a rule says "leave this
     ;; alone" without needing a way to say it.
     ((nth 2 state) (current-indentation))
     ((= 0 (nth 0 state)) 0)
     ;; Element 4, not element 1: the delimiter rather than the start of the
     ;; expression. They differ by any prefix, and `'(a b)' lines its body up
     ;; against the parenthesis, not against the quote.
     (t (lisp-indent-under (nth 4 state))))))

(defun lisp-indent-under (open)
  "Return the column for a line inside the list whose delimiter is at OPEN."
  (let* ((column (progn (goto-char open) (current-column)))
         (head (lisp-head-after open)))
    (cond
     ;; (defun f () ...) -- a form whose body is indented by a fixed amount
     ;; rather than lined up under an argument.
     ((and head (get head 'lisp-indent)) (+ column 2))
     ;; ((a b) ...) -- the head is itself a list, so there is no name to line
     ;; anything up under. Just inside the delimiter.
     ((null head) (+ column 1))
     ;; (foo bar
     ;;      baz) -- line up under the first argument, when it shares a line
     ;; with the head. When it does not there is nothing to line up with.
     (t (or (lisp-argument-column open head) (+ column 1))))))

(defun lisp-head-after (open)
  "Return the name just inside the list at OPEN, as a string, or nil when
another list starts there."
  (goto-char (+ open 1))
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if (and bounds (= (nth 0 bounds) (+ open 1)))
        (buffer-substring (nth 0 bounds) (nth 1 bounds)))))

(defun lisp-argument-column (open head)
  "Return the column of the first argument after HEAD in the list at OPEN,
or nil when there is none on OPEN's line."
  ;; OPEN's line first, because everything below moves point and there is no
  ;; way to ask a position's line without going there. `forward-sexp' lands at
  ;; the argument's end, so its start is forward and then back; and it does
  ;; nothing at the end of a list, so a point that did not move means there
  ;; was no argument.
  (let ((open-line (progn (goto-char open) (line-number-at-point))))
    (goto-char (+ open 1 (length head)))
    (let ((before (point)))
      (forward-sexp)
      (if (= before (point))
          nil
          (progn
            (backward-sexp)
            (if (= (line-number-at-point) open-line)
                (current-column)))))))

;; The forms whose bodies are indented by a fixed amount rather than lined up
;; under an argument. On the symbols themselves, globally, exactly as Emacs
;; keeps `lisp-indent-function' -- a language's shape is a property of its
;; names, not of one mode that happens to read them.
(dolist (name '("defun" "defmacro" "defcommand" "lambda" "let" "let*" "when"
                "unless" "while" "dolist" "dotimes" "progn" "cond" "if"
                "condition-case" "unwind-protect" "catch" "with-current-buffer"))
  (put name 'lisp-indent 2))

(defcommand open-line-below (count) ("p")
  "Open COUNT blank lines below this one, and leave point on the first,
indented."
  (end-of-line)
  ;; Where the first new line will begin, taken before the text moves: after
  ;; COUNT newlines point is on the *last* of them, and stepping back by lines
  ;; would have to care about how `previous-line' treats a goal column.
  (let ((start (+ (point) 1)))
    (dotimes (n count)
      (insert "\n"))
    (goto-char start)
    (indent-line)))

(defcommand open-line-above (count) ("p")
  "Open COUNT blank lines above this one, and leave point on the first,
indented."
  ;; From the start of this line, rather than below the line above: the first
  ;; line has none.
  (beginning-of-line)
  (let ((start (point)))
    (dotimes (n count)
      (insert "\n"))
    (goto-char start)
    (indent-line)))

;; ---------------------------------------------------------------------------
;; Enter
;; ---------------------------------------------------------------------------

(defvar electric-indent-mode t
  "Whether Return indents the line it opens, as `newline-and-indent' does.")

(defun indent--delete-trailing-space ()
  "Delete the spaces and tabs just before point, on this line only."
  (let* ((end (point))
         (start (progn (beginning-of-line) (point)))
         (gap (string-match-p "[ \t]+$" (buffer-substring start end))))
    (goto-char end)
    (when gap
      (delete-region (+ start gap) end))))

(defcommand newline-and-indent () nil
  "Delete the spaces just before point, insert a newline, and indent the new
line as Tab would."
  (indent--delete-trailing-space)
  (insert-newline)
  (indent-line))

(defcommand newline () nil
  "Insert a newline, and indent the new line when `electric-indent-mode' is
on."
  (if electric-indent-mode
      (newline-and-indent)
      (insert-newline)))

;; Bound here rather than in common-keymaps.lisp, beside Tab, because of what
;; happens when this file is not loaded. Without it the binding would name a
;; command that does not exist and Enter would stop working altogether; with
;; it here, Enter simply stays the built-in `insert-newline'.
(define-key nil "<ret>" 'newline)

;; Shared readers for the programming-language indenters. They only inspect
;; text: indent-line restores point and performs the actual edit.

(defun prog-indent--trim (text)
  "Return TEXT without horizontal whitespace at either end."
  (let* ((start (or (string-match-p "[^ \t]" text) (length text)))
         (end (if (string-match "[ \t]+$" text) (match-beginning 0) (length text))))
    (substring text start (max start end))))

(defun prog-indent--char (at)
  "Return the character at AT as a string, or an empty string at buffer end."
  (if (and (>= at 0) (< at (point-max)))
      (buffer-substring at (+ at 1)) ""))

(defun prog-indent--code ()
  "Return this line with strings and comments masked, retaining its offsets."
  (beginning-of-line)
  (let* ((start (point)) (text (current-line)) (size (length text))
         (at 0) (out "") (state (syntax-ppss start)))
    (while (< at size)
     (let ((before at))
      (cond
       ((nth 2 state)
        (goto-char (nth 2 state))
        (let ((before (point)))
          (forward-sexp)
          (let ((end (if (> (point) before) (min size (- (point) start)) size)))
            (setq end (max (+ at 1) end))
            ;; A literal is still an expression, even when it is the whole line.
            (setq out (concat out "0" (make-string (- end at 1) " ")) at end))))
       ((nth 3 state)
        (let* ((open (nth 3 state))
               (block (string= (buffer-substring open (min (+ open 2) (point-max))) "/*"))
               (close (if block (string-match-p "\\*/" text at)))
               (end (if close (+ close 2) size)))
          (setq out (concat out (make-string (- end at) " ")) at end)))
       (t
        (let ((token (string-match-p "[\"'`/#]" text at)))
          (if (null token)
              (setq out (concat out (substring text at)) at size)
              (progn
                (setq out (concat out (substring text at token)) at token)
                (setq state (syntax-ppss (+ start at 1)))
                (unless (or (nth 2 state) (nth 3 state))
                  ;; The scanner treats a C character literal as an atom, so
                  ;; it does not report an interior string state for it.
                  (if (and (get (major-mode) 'indent-char-literals)
                           (string-match "^'(\\\\.|[^'\\\\])'" (substring text at)))
                      (let ((count (match-end 0)))
                        (setq out (concat out "0" (make-string (- count 1) " "))
                              at (+ at count)))
                      (setq out (concat out (substring text at (+ at 1))) at (+ at 1)))))))))
      (when (and (> at before) (< at size)) (setq state (syntax-ppss (+ start at))))))
    (goto-char start)
    out))

(defun prog-indent--previous ()
  "Return the preceding code line as (START COLUMN CODE), skipping blank lines."
  (beginning-of-line)
  (let ((found nil))
    (while (and (not found) (> (point) (point-min)))
      (backward-char)
      (beginning-of-line)
      (let* ((start (point)) (column (current-indentation))
             (code (prog-indent--trim (prog-indent--code))))
        (unless (string= code "") (setq found (list start column code)))))
    found))

(defun prog-indent--under (open closing)
  "Return a continuation column inside OPEN, or its line's column if CLOSING."
  (goto-char open)
  (let* ((base (current-indentation))
         (line-start (progn (beginning-of-line) (point)))
         (code (prog-indent--code))
         (rest (substring code (+ 1 (- open line-start))))
         (first (string-match-p "[^ \t]" rest)))
    (cond (closing base)
          (first (+ (- open line-start) 1 first))
          (t (+ base (indent-width))))))



(log "End of the indent.lisp")
(provide 'indent)
