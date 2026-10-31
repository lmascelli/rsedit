;;; risp-repl --- a buffer to evaluate the editor's Lisp in, as *scratch* is in Emacs.
;;;
;;; M-x risp-repl opens *risp*, a risp-mode buffer where C-j evaluates the
;;; block of expressions written last and writes its value under it.
;;; User manual: the `repl' page.
;;; Design: `doc/architettura-editor.typ, <mod-risp-repl>`.

(require 'commands)
(require 'debug)
(require 'risp-mode)

;; Not a major mode: only the keymap that `set-local-keymap' gives a buffer,
;; which keeps risp-mode's colouring and indentation.
(make-mode 'risp-repl-map)

(defconst risp-repl-buffer "*risp*"
  "The name of the buffer `risp-repl' opens.")

;; A blank line, or a line a value was written on, ends a block. The second is
;; matched in the gap between two expressions, where `backward-sexp' skips it.
(defconst risp-repl--separator "\\n[ \\t]*\\n|;+ *(=>|!!)"
  "The regexp that, found between two expressions, puts them in two blocks.")

;; ---------------------------------------------------------------------------
;; Finding the block
;; ---------------------------------------------------------------------------

(defun risp-repl--separated-p (from to)
  "Non-nil when the expression at FROM and the text at TO are in two blocks."
  (goto-char from)
  (forward-sexp)
  (string-match-p risp-repl--separator (buffer-substring (point) to)))

(defun risp-repl--block-start (end)
  "The start of the block of expressions that ends at END: END when none does."
  (goto-char end)
  (backward-sexp)
  (let ((start (point))
        (done (>= (point) end)))
    ;; Inside a list only the expression before END counts, as in Emacs: its
    ;; neighbours are arguments, not a program.
    (when (> (car (syntax-ppss end)) 0)
      (setq done t))
    (while (not done)
      (goto-char start)
      (backward-sexp)
      (let ((here (point)))
        (if (or (>= here start) (risp-repl--separated-p here start))
            (setq done t)
            (setq start here))))
    (goto-char end)
    (if (>= start end) end start)))

(defun risp-repl--forms (start end)
  "The texts of the expressions between START and END, in order."
  (let ((forms nil)
        (done nil))
    (goto-char start)
    (while (not done)
      (let ((from (point)))
        (forward-sexp)
        (if (or (<= (point) from) (> (point) end))
            (setq done t)
            (let ((to (point)))
              ;; Back over it, so that the comments before it stay out.
              (backward-sexp)
              (setq forms (cons (buffer-substring (point) to) forms))
              (goto-char to)))))
    (goto-char end)
    (reverse forms)))

;; ---------------------------------------------------------------------------
;; Evaluating it, and writing the value
;; ---------------------------------------------------------------------------

(defun risp-repl--evaluate (forms)
  "The outcome of evaluating FORMS in turn: (t VALUE) of the last, or (nil WHY)
of the first that fails, after which none is evaluated."
  (let ((outcome nil))
    (while forms
      (setq outcome (eval-string-top-level (car forms)))
      (setq forms (if (car outcome) (cdr forms) nil)))
    outcome))

(defun risp-repl--join (parts separator)
  "PARTS, a list of strings, joined into one with SEPARATOR between them."
  (let ((joined (car parts)))
    (dolist (part (cdr parts))
      (setq joined (concat joined separator part)))
    (or joined "")))

(defun risp-repl--print (value)
  "VALUE written as the reader reads it back: a string in quotes."
  (if (stringp value)
      (concat "\""
              (risp-repl--join
               (split-string (risp-repl--join (split-string value "\\") "\\\\")
                             "\"")
               "\\\"")
              "\"")
      (format "%s" value)))

(defun risp-repl--result-line (outcome)
  "The comment that shows OUTCOME, one line of it for each line of the value."
  (concat (if (car outcome) ";; => " ";; !! ")
          (risp-repl--join
           (split-string (if (car outcome)
                             (risp-repl--print (nth 1 outcome))
                             (nth 1 outcome))
                         "\n")
           "\n;;    ")))

(defcommand risp-repl-eval-print-block () nil
  "Evaluate the block of expressions before point, one after the other, and
write the value of the last, or the error that stopped them, on a comment line
under it."
  (let ((ppss (syntax-ppss))
        (buffer (current-buffer))
        (end (point)))
    (if (or (nth 2 ppss) (nth 3 ppss))
        (message "Point is in a string or a comment")
        (let ((forms (risp-repl--forms (risp-repl--block-start end) end)))
          (if (null forms)
              (message "No expression before point")
              (let ((line (risp-repl--result-line (risp-repl--evaluate forms))))
                ;; What was evaluated may have made another buffer current,
                ;; and the line does not belong in it.
                (if (string= (current-buffer) buffer)
                    (progn (goto-char end)
                           (insert "\n" line "\n"))
                    (message "%s" line))))))))

;; ---------------------------------------------------------------------------
;; The buffer
;; ---------------------------------------------------------------------------

(defcommand risp-repl-mode () nil
  "Give this buffer the keys of the REPL, C-j above all, or take them away
when it has them. The buffer keeps its mode and its colouring."
  (if (eq (buffer-get 'local-keymap) 'risp-repl-map)
      (progn (set-local-keymap nil)
             (message "REPL keys off"))
      (progn (set-local-keymap 'risp-repl-map)
             (message "C-j evaluates the block before point"))))

(defcommand risp-repl () nil
  "Switch to the *risp* buffer, made the first time in risp-mode and with the
keys of the REPL, where C-j evaluates the expressions written last."
  (if (member risp-repl-buffer (all-buffer-names))
      (switch-to-buffer risp-repl-buffer)
      (progn
        (buffer-create risp-repl-buffer 'risp-mode)
        (switch-to-buffer risp-repl-buffer)
        (set-local-keymap 'risp-repl-map)
        (insert ";; Lisp, evaluated in the running editor. Write an expression,\n"
                ";; or a few, and press C-j after them: the value comes below.\n\n"))))

(define-key 'risp-repl-map "C-j" 'risp-repl-eval-print-block)

(log "risp-repl loaded")
(provide 'risp-repl)
