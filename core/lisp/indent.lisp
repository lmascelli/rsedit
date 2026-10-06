;;; indent --- what Tab does.
;;;
;;; The editor has no opinion about how any language should be indented, and
;;; this file has only one: Tab indents the region if there is one, otherwise
;;; the line, and if the line was already where it belongs it does whatever
;;; `tab-always-indent' says. The key is never dead.
;;;
;;; # Where a language's opinion goes
;;;
;;;   (put 'risp-mode 'indent-function 'lisp-indent-line)
;;;
;;; The function takes no arguments, is called with point on the line in
;;; question, and returns the *column* that line should start at. It does not
;;; touch the buffer -- `indent-line-to' does that, through the same two doors
;;; every other edit goes through, so indenting is one undo step and a
;;; read-only buffer refuses it.
;;;
;;; Returning a column rather than doing the work is the load-bearing decision.
;;; A mode can only be wrong about a number; `indent-region' can call it a
;;; hundred times without a hundred chances to mangle a line; and where point
;;; ends up afterwards is written once, in Rust, instead of once per language.
;;;
;;; A mode that declares nothing gets `previous-indentation': match the line
;;; above. Language-agnostic, and right often enough to be worth having in a
;;; buffer whose mode nobody has written yet.
;;;
;;; # A caveat about columns
;;;
;;; `current-indentation' counts characters and `current-column' counts
;;; columns. They agree in a buffer indented with spaces, and `indent-line-to'
;;; only ever writes spaces -- so they stay in step unless someone types a
;;; literal tab, at which point the arithmetic here is off by the difference.

(setq tab-width 4)

;; What Tab does when the line is *already* indented correctly. Emacs' name and
;; Emacs' values, so it means what it looks like:
;;
;;   nil         insert `tab-width' spaces (the default here)
;;   t           nothing; Tab only ever indents
;;   'complete   ask `completion-at-point', which is Emacs' default for
;;               programming modes and costs nothing now that capf exists
(setq tab-always-indent nil)

(defun indent-width ()
  "How wide a Tab is here: the mode's own, or the global default.

Per mode because two languages in one editor disagree about this more often
than they agree, and `(put 'rust-mode 'tab-width 4)' is where that belongs."
  (or (get (major-mode) 'tab-width) tab-width))

(defun indent-line ()
  "Indent the current line the way its mode wants.

Returns t if the line moved, nil if it was already there -- which is what lets
`indent-for-tab-command' tell \"I indented it\" from \"there was nothing to
do\"."
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
  "The first and last line numbers the region covers, as (FROM TO).

A region that ends at the very start of a line does not cover that line. That is
what selecting whole lines from the left margin produces -- the end lands at
column 0 of the line *after* the last one meant -- and shifting a line nobody
selected is the surprise this rule exists to avoid."
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
  "The column this line's text starts at, or nil if the line is blank.

Columns rather than `current-indentation''s characters, so that a line indented
with a literal tab is shifted from where it *looks* indented to, not from one."
  (back-to-indentation)
  (let ((column (current-column))
        (here (point)))
    (end-of-line)
    (if (= here (point)) nil column)))

(defun indent-rigidly-lines (from to amount)
  "Shift lines FROM to TO by AMOUNT columns, keeping their shape.

Every line moves by the same amount, so whatever the indentation inside the
block said -- this is nested under that -- still says it afterwards. A
negative AMOUNT moves left and stops at the margin: a line with less
indentation than that goes to column 0 rather than taking the others with it.

Blank lines are left alone. Shifting one would only give it trailing
whitespace."
  (dotimes (n (+ 1 (- to from)))
    (goto-line (+ from n))
    (let ((column (indent--text-column)))
      (when column
        (indent-line-to (max 0 (+ column amount)))))))

(defvar indent-region-style 'auto
  "What Tab does to a region: `reindent', `shift' or `auto'.

  reindent  put every line where the mode's `indent-function' says
  shift     move the block right by one `indent-width', keeping its shape
  auto      reindent in a mode that has an `indent-function', shift in one
            that does not

`auto' is the default because reindenting without a rule means matching the
line above, and applied line by line that *flattens* a block: each line copies
the one before it, so a nested body lands at the same column as its header.

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
  "Indent every line the region touches, and leave the region in place.

Reindent each line, or shift the block as a whole, as `indent-region-style'
says."
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
  "Shift the region, or the current line, by one `indent-width' in DIRECTION.

DIRECTION is 1 for right and -1 for left. Without a region the line point is on
is the block, and point keeps its place in the text."
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
  "Shift the region -- or the line -- right by one `indent-width'.

Unlike Tab this never asks the mode where the lines belong: the block moves as
it is."
  (indent--shift 1))

(defcommand indent-rigidly-left () nil
  "Shift the region -- or the line -- left by one `indent-width'.

Lines stop at the margin rather than holding the rest back, so a block whose
lines are indented by different amounts still moves as far as it can."
  (indent--shift -1))

(defcommand indent-for-tab-command () nil
  "Indent the region, or the line, or fall back to `tab-always-indent'.

The three cases in order. With a region, indent all of it. Without one, indent
the line -- and if that changed nothing, the line was already where its mode
wants it, so `tab-always-indent' decides what the keystroke means instead.

That last case is the one worth stating: a Tab that did nothing on an
already-indented line reads as broken, and a Tab that always inserted would
make the first case unreachable."
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
;; Not attached to anything yet: there is no `risp-mode' in the editor, so this
;; ships as a rule waiting for a mode. When one lands it is one line --
;; (put 'risp-mode 'indent-function 'lisp-indent-line) -- and it works for any
;; mode whose syntax table pairs parentheses.

(defun lisp-indent-line ()
  "The column the current line should start at, in a Lisp.

Reads the structure rather than the text: `syntax-ppss' says how deep point is
and where its enclosing list opened, and everything below follows from those
two. That is why this needs no regular expressions and does not care what the
language's keywords are."
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
  "The column for a line inside the list whose delimiter is at OPEN."
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
  "The symbol just inside OPEN, as a string, or nil if a list starts there.

A string rather than a symbol because properties are keyed by name, so
`(get head 'lisp-indent)' works on it directly and nothing has to intern."
  (goto-char (+ open 1))
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if (and bounds (= (nth 0 bounds) (+ open 1)))
        (buffer-substring (nth 0 bounds) (nth 1 bounds)))))

(defun lisp-argument-column (open head)
  "The column of the first argument after HEAD, if it is on OPEN's line.

nil when the head is the last thing in the form, or when the first argument is
already on a line of its own -- in both cases there is nothing on that line to
line up under.

`forward-sexp' lands at an expression's *end*, so getting to its start means
going forward and then back. Comparing point before and after is how this asks
\"was there anything there at all\", since the motion refuses rather than
signals at the end of a list."
  ;; OPEN's line first, because everything below moves point and there is no
  ;; way to ask a position's line without going there.
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

;; Insert newline above or below and move the cursor to them
(defcommand open-line-below (count) ("p")
  "Open COUNT blank lines below this one and leave point on the first.

Indented the way the mode wants, because a blank line at column zero in code
is a line you have to fix before you can use it."
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
  "Open COUNT blank lines above this one and leave point on the first.

The mirror of `open-line-below'. Written as \"go to the start of this line and
insert there\" rather than \"go up a line and open below it\", because there is
no line above the first one and the second form would quietly do nothing there."
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
  "Whether Enter indents the line it opens, as `newline-and-indent' does.

On by default, as in Emacs. (setq electric-indent-mode nil) makes Enter a plain
newline.

A variable rather than a binding to change, because the binding is not the
user's to keep track of -- `<ret>' is bound to `newline' once, in this file,
and this decides what that means. Read at every keystroke, so setting it takes
effect at once.")

(defun indent--delete-trailing-space ()
  "Delete the spaces and tabs just before point, on this line only.

What Enter leaves behind on a line whose indentation it opened from: pressing it
on an indented blank line would otherwise leave that line made of nothing but
spaces."
  (let* ((end (point))
         (start (progn (beginning-of-line) (point)))
         (gap (string-match-p "[ \t]+$" (buffer-substring start end))))
    (goto-char end)
    (when gap
      (delete-region (+ start gap) end))))

(defcommand newline-and-indent () nil
  "Insert a newline, and indent the new line the way its mode wants.

The indentation comes from the same place Tab's does -- the mode's
`indent-function', or the line above -- so a mode that teaches Tab where its
lines go has taught Enter as well.

Whitespace just before point goes first. Splitting `foo   |bar' should not leave
`foo   ' behind, and Enter on an indented blank line should not leave a line of
spaces above the new one. Text after point keeps its place: the newline carries
it to the new line, and `indent-line-to' replaces whatever whitespace it brought
with the indentation that line should have."
  (indent--delete-trailing-space)
  (insert-newline)
  (indent-line))

(defcommand newline () nil
  "What Enter does: `newline-and-indent' if `electric-indent-mode' is on, a
plain newline if it is not.

Asked at every keystroke rather than when the key was bound, so setting the
variable takes effect immediately and needs no rebinding."
  (if electric-indent-mode
      (newline-and-indent)
      (insert-newline)))

;; Bound here rather than in common-keymaps.lisp, beside Tab, because of what
;; happens when this file is not loaded. Without it the binding would name a
;; command that does not exist and Enter would stop working altogether; with
;; it here, Enter simply stays the built-in `insert-newline'.
(define-key nil "<ret>" 'newline)

(log "End of the indent.lisp")
