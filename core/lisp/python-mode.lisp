;;; python-mode --- Python colouring, completion vocabulary and indentation.

(make-mode 'python-mode)

;; Triple quotes precede the one-quote regions: they start at the same column.
(add-syntax-region 'python-mode "(?i:[rubf]{0,2})\"\"\"" "\"\"\"" 'string "\\\\.")
(add-syntax-region 'python-mode "(?i:[rubf]{0,2})'''" "'''" 'string "\\\\.")
(add-syntax-region 'python-mode "(?i:[rubf]{0,2})\"" "\"" 'string "\\\\.")
(add-syntax-region 'python-mode "(?i:[rubf]{0,2})'" "'" 'string "\\\\.")
(add-syntax-rule 'python-mode "#.*" 'comment)

(defconst python-keywords
  '("and" "as" "assert" "async" "await" "break" "class" "continue" "def"
    "del" "elif" "else" "except" "finally" "for" "from" "global" "if"
    "import" "in" "is" "lambda" "nonlocal" "not" "or" "pass" "raise"
    "return" "try" "while" "with" "yield" "match" "case")
  "Python keywords and structural-pattern soft keywords.")
(defconst python-constants '("True" "False" "None" "Ellipsis" "NotImplemented")
  "Python constants offered for completion.")
(defconst python-builtins
  '("abs" "all" "any" "ascii" "bin" "breakpoint" "callable" "chr" "compile"
    "delattr" "dir" "divmod" "enumerate" "eval" "exec" "filter" "format"
    "getattr" "globals" "hasattr" "hash" "help" "hex" "id" "input"
    "isinstance" "issubclass" "iter" "len" "locals" "map" "max" "min"
    "next" "oct" "open" "ord" "pow" "print" "repr" "reversed" "round"
    "setattr" "sorted" "sum" "super" "vars" "zip" "__import__")
  "Python built-in functions offered for completion.")
(defconst python-types
  '("bool" "bytearray" "bytes" "classmethod" "complex" "dict" "float"
    "frozenset" "int" "list" "memoryview" "object" "property" "range"
    "set" "slice" "staticmethod" "str" "tuple" "type"
    "BaseException" "Exception" "ValueError" "TypeError" "RuntimeError"
    "KeyError" "IndexError" "OSError" "StopIteration")
  "Python built-in types offered for completion.")
(put 'python-mode 'keywords
     (append python-keywords (append python-constants (append python-builtins python-types))))

(add-syntax-rule 'python-mode "@[A-Za-z_][A-Za-z0-9_.]*" 'attribute)
(add-syntax-rule 'python-mode (concat "\\b" (regexp-opt python-keywords) "\\b") 'keyword)
(add-syntax-rule 'python-mode (concat "\\b" (regexp-opt python-constants) "\\b") 'builtin)
(add-syntax-rule 'python-mode (concat "\\b" (regexp-opt python-types) "\\b") 'type)
(add-syntax-rule 'python-mode (concat "\\b" (regexp-opt python-builtins) "\\b") 'builtin)
(add-syntax-rule 'python-mode "\\b[A-Z][A-Za-z0-9_]*\\b" 'type)
(add-syntax-rule 'python-mode "\\b(?:0[xX][0-9a-fA-F_]+|0[bB][01_]+|0[oO][0-7_]+|[0-9][0-9_]*(?:\\.[0-9_]*)?(?:[eE][+-]?[0-9_]+)?)[jJ]?\\b|\\.[0-9][0-9_]*(?:[eE][+-]?[0-9_]+)?[jJ]?\\b" 'builtin)
(add-syntax-rule 'python-mode "\\b([\\p{L}_][\\p{L}\\p{N}_]*)[ \t]*\\(" 'function 1)
(set-face 'attribute "bright-yellow" nil)

(set-syntax-pairs 'python-mode "()[]{}")
(set-syntax-entry 'python-mode "'" 'string)
(set-comment-syntax 'python-mode '(("#")))
(set-string-syntax 'python-mode '(("\"\"\"" "\"\"\"" t) ("'''" "'''" t)))
(put 'python-mode 'tab-width 4)
(put 'python-mode 'indent-function 'python-indent-line)

(defun python--clause-column (code limit)
  "Find a matching compound statement for CODE before point, no deeper than LIMIT."
  (let* ((pattern (cond
                   ((string-match-p "^elif\\b" code) "^(async[ \t]+)?(if|elif)\\b")
                   ((string-match-p "^(except|finally)\\b" code) "^(try|except)\\b")
                   ((string-match-p "^case\\b" code) "^(match|case)\\b")
                   (t "^(if|elif|for|while|try|except|async[ \t]+for)\\b")))
         (line (prog-indent--previous)) (answer nil))
    (while (and line (null answer))
      (when (and (<= (nth 1 line) limit) (string-match-p pattern (nth 2 line)))
        (setq answer (+ (nth 1 line)
                        (if (and (string-match-p "^case\\b" code)
                                 (string-match-p "^match\\b" (nth 2 line))) (indent-width) 0))))
      (unless answer (setq line (prog-indent--previous))))
    answer))

(defun python-indent-line ()
  "(python-indent-line): Return this Python line's indentation; see `indentation'."
  (beginning-of-line)
  (let* ((start (point)) (existing (current-indentation)) (state (syntax-ppss))
         (code (prog-indent--trim (prog-indent--code)))
         (open (nth 4 state)))
    (cond
     ((nth 2 state) existing)
     (open (prog-indent--under open (string-match-p "^[)\\]}]" code)))
     (t
      (goto-char start)
      (let* ((previous (prog-indent--previous))
             (base (if previous (nth 1 previous) 0))
             (before (if previous (nth 2 previous) ""))
             (statement base))
        ;; A compound header can close its argument list on another line.
        (when previous
          (goto-char (nth 0 previous))
          (let ((earlier (prog-indent--previous)))
            (while (and earlier (string-match-p "\\\\[ \t]*$" (nth 2 earlier)))
              (setq statement (nth 1 earlier) earlier (prog-indent--previous))))
          (let ((outer (nth 4 (syntax-ppss (nth 0 previous)))))
            (when outer (goto-char outer) (setq statement (current-indentation)))))
        (cond
         ((string-match-p "^(elif|else|except|finally|case)\\b" code)
          (goto-char start)
          (or (python--clause-column code base) (max 0 (- base (indent-width)))))
         ((string-match-p ":[ \t]*$" before) (+ statement (indent-width)))
         ((string-match-p "\\\\[ \t]*$" before) (+ statement (indent-width)))
         ;; Indentation is Python syntax: keep a written dedent instead of
         ;; moving a following top-level statement back into the last body.
         ((and (not (string= code "")) (< existing base)) existing)
         ((and (string= code "") (string-match-p "^(return|raise|break|continue|pass)\\b" before))
          (max 0 (- base (indent-width))))
         (t statement)))))))

(add-auto-mode "\\.(py|pyw|pyi)$" 'python-mode)
(log "python-mode loaded")
