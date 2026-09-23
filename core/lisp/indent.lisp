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

(defun indent-region ()
  "Indent every line the region touches, and leave the region in place.

Walked by line *number*, not by position: indenting a line changes every offset
below it, so a loop that remembered where the region ended would be indenting
the wrong text by the third line.

The region is re-established at the end rather than preserved, because it
cannot be preserved -- every edit deactivates the mark at the chokepoint, so it
is gone by the second line. Putting it back is what makes pressing Tab twice
work on the same block."
  ;; Both ends first, before anything moves. The region is the span between
  ;; mark and point, so `(goto-char (region-beginning))' collapses it -- and
  ;; `region-end' then answers with the mark rather than the end that was
  ;; there a moment ago.
  (let* ((start (region-beginning))
         (end (region-end))
         (from (progn (goto-char start) (line-number-at-point)))
         (to (progn (goto-char end) (line-number-at-point))))
    (dotimes (n (+ 1 (- to from)))
      (goto-line (+ from n))
      (indent-line))
    (goto-line from)
    (beginning-of-line)
    (set-mark)
    (goto-line to)
    (end-of-line)))

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

(log "End of the indent.lisp")
