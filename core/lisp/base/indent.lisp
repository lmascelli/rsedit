;;; indent --- what Tab and Return do.
;;;
;;; Rust supplies the line and Tab commands, and Return's `newline', which only
;;; inserts. This module supplies region operations, language-rule helpers and
;;; `electric-indent-mode', the minor mode that indents as you type.
;;; User manual: `indentation'.
;;; Design: `doc/architettura-editor.typ, <mod-indent>`.

(require 'commands)

(defvar tab-always-indent t
  "How Tab handles indentation and text: t, as in Emacs, always indents the
line, wherever point is on it; complete also tries `completion-at-point' when
indentation and point did not change; nil indents in leading whitespace and
inserts spaces up to the next tab stop after text. Consecutive Tabs cycle a
mode's indent-levels-function alternatives. Without alternatives, consecutive
Tabs with nil insert spaces.")

;; ---------------------------------------------------------------------------
;; Regions
;; ---------------------------------------------------------------------------
;;
;; By line number, since indenting a line moves every offset below it; and the
;; region is selected again at the end, since every edit deactivates the mark.

(defun indent--region-lines ()
  "The first and last line numbers the region covers, as (FROM TO)."
  ;; Both ends first: moving point moves one end of the region. A region that
  ;; ends at the start of a line does not cover that line.
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
     ;; `auto', and anything not understood: a typo gets the sensible
     ;; behaviour, not an error on every Tab.
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
        ;; Point is put back first, so that it keeps its character. A blank line
        ;; shifts here, unlike in a region: it is the one asked about.
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

;; ---------------------------------------------------------------------------
;; Indenting a Lisp
;; ---------------------------------------------------------------------------
;;
;; The rule risp-mode uses. It reads the structure from `syntax-ppss', not the
;; text, and works in any mode whose syntax table pairs parentheses.

(defun lisp-indent-line ()
  "Return the column the line point is on should start at, in a Lisp."
  (let ((state (syntax-ppss)))
    (cond
     ;; In a string the leading whitespace is content: the line stays as it is.
     ((nth 2 state) (current-indentation))
     ((= 0 (nth 0 state)) 0)
     ;; Element 4, the delimiter, not 1, the expression: `'(a b)' lines its body
     ;; up against the parenthesis, not the quote.
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
     ;;      baz) -- under the first argument, when it shares the head's line.
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
  ;; Forward and back to the argument's start; a point that did not move
  ;; means there was no argument.
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

;; The forms whose bodies are indented by a fixed amount, on the symbols
;; themselves, as Emacs keeps `lisp-indent-function'.
(dolist (name '("defun" "defmacro" "defcommand" "lambda" "let" "let*" "when"
                "unless" "while" "dolist" "dotimes" "progn" "cond" "if"
                "condition-case" "unwind-protect" "catch" "with-current-buffer"))
  (put name 'lisp-indent 2))

(defcommand open-line-below (count) ("p")
  "Open COUNT blank lines below this one, and leave point on the first,
indented."
  (end-of-line)
  ;; Where the first new line begins, taken before the text moves: after COUNT
  ;; newlines point is on the last of them.
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
;; Indenting as you type
;; ---------------------------------------------------------------------------
;;
;; Return runs `newline', which only inserts, and the editor runs
;; `post-self-insert-hook' after it as after a typed character. The minor mode
;; hangs there, as Emacs' does: whatever is on Return, a newline that reaches
;; the buffer is indented, and turning the mode off leaves it plain.

(define-minor-mode electric-indent-mode nil
  "Indent as you type in the current buffer: Return leaves the line it ends
without trailing blanks and indented again, and indents the line it opens; a
character listed in the mode's electric-indent-chars property indents its line
when it is the first on it. On in every buffer unless turned off.")

(defun electric-indent--blank-p (text)
  "Whether TEXT holds nothing but spaces and tabs."
  (if (string-match-p "^[ \t]*$" text) t nil))

(defun electric-indent--leave-line ()
  "Tidy the line point is on, which Return has just ended, and return the
column it chose when it was blank, nil otherwise."
  (beginning-of-line)
  (let ((text (current-line)))
    (if (electric-indent--blank-p text)
        ;; Blank: the indentation chosen there, by Tab or a level picked, is
        ;; kept for the next line, column 0 included, and the blank line
        ;; itself left empty.
        (let ((column (length text)))
          (indent-line-to 0)
          column)
        (progn
          ;; The blanks before where Return was pressed.
          (if (string-match "[ \t]+$" text)
              (let ((from (+ (point) (match-beginning 0))))
                (delete-region from (+ (point) (length text)))))
          ;; Indented again, as Emacs does, where the mode has a rule and does
          ;; not ask to be left alone: a word typed may have changed where the
          ;; line belongs, as `else' or `}' do.
          (if (and (get (major-mode) 'indent-function)
                   (not (get (major-mode) 'electric-indent-inhibit)))
              (indent-line))
          nil))))

(defun electric-indent--after-newline ()
  "Indent the line Return has just opened, after tidying the one it ended."
  ;; By line number: tidying the line above moves every offset below it.
  (let* ((opened (line-number-at-point))
         (kept (progn (goto-char (- (point) 1)) (electric-indent--leave-line))))
    (goto-line opened)
    (beginning-of-line)
    (if kept (indent-line-to kept) (indent-line))))

(defun electric-indent--after-char (typed)
  "Indent the line TYPED, just before point, is on, when the mode lists it in
its electric-indent-chars property, it is the first on its line, and it is
code."
  (let ((chars (get (major-mode) 'electric-indent-chars))
        (at (- (point) 1)))
    (if (and (member typed chars)
             (get (major-mode) 'indent-function)
             ;; What is before TYPED on its line, TYPED being the one
             ;; character before point.
             (electric-indent--blank-p
              (substring (current-line) 0 (max 0 (- (current-column) 1)))))
        (let ((state (syntax-ppss at)))
          (unless (or (nth 2 state) (nth 3 state))
            (indent-line))))))

(defun electric-indent-post-self-insert ()
  "Indent after Return, or after an electric character. Run from
`post-self-insert-hook' where `electric-indent-mode' is on."
  (if (> (point) (point-min))
      (let ((typed (buffer-substring (- (point) 1) (point))))
        (if (string= typed "\n")
            (electric-indent--after-newline)
            (electric-indent--after-char typed)))))

(add-hook 'electric-indent-mode "post-self-insert-hook"
          'electric-indent-post-self-insert)

;; In every buffer from now on, whatever its major mode, and in those already
;; open, which `minor-mode-enable' leaves as they are.
(minor-mode-enable 'electric-indent-mode)
(dolist (name (all-buffer-names))
  (minor-mode-on 'electric-indent-mode name))

;; ---------------------------------------------------------------------------
;; Readers for the language modes' rules
;; ---------------------------------------------------------------------------
;;
;; They only read: `indent-line' puts point back and makes the edit.

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
