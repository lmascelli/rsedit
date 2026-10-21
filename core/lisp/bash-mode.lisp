;;; bash-mode --- shell colouring and indentation, including literal heredocs.
;;;
;;; For the files whose names end in .sh and .bash, and for .bashrc, .profile
;;; and Bash's other startup files. User manual: the `modes' page, and
;;; `indentation' for how it indents.

(make-mode 'bash-mode)
(put 'bash-mode 'heredoc-delimiters nil)
(add-syntax-region 'bash-mode "\\$'" "'" 'string "\\\\.")
(add-syntax-region 'bash-mode "'" "'" 'string)
(add-syntax-region 'bash-mode "\"" "\"" 'string "\\\\.")
(add-syntax-region 'bash-mode "`" "`" 'string "\\\\.")
;; A # inside a word or ${#parameter} is not a comment.
(add-syntax-rule 'bash-mode "(^|[ \t;|&()])(#.*)" 'comment 2)
(defconst bash-keywords
  '("if" "then" "elif" "else" "fi" "for" "while" "until" "do" "done"
    "case" "in" "esac" "select" "function" "time" "coproc")
  "Shell reserved words offered for completion.")
(defconst bash-builtins
  '("alias" "bg" "bind" "break" "builtin" "cd" "command" "compgen"
    "complete" "continue" "declare" "dirs" "disown" "echo" "enable" "eval"
    "exec" "exit" "export" "false" "fc" "fg" "getopts" "hash" "help"
    "history" "jobs" "kill" "let" "local" "logout" "mapfile" "popd"
    "printf" "pushd" "pwd" "read" "readarray" "readonly" "return" "set"
    "shift" "shopt" "source" "test" "times" "trap" "true" "type"
    "typeset" "ulimit" "umask" "unalias" "unset" "wait")
  "Bash built-in commands offered for completion.")
(put 'bash-mode 'keywords (append bash-keywords bash-builtins))
(add-syntax-rule 'bash-mode (concat "\\b" (regexp-opt bash-keywords) "\\b") 'keyword)
(add-syntax-rule 'bash-mode (concat "\\b" (regexp-opt bash-builtins) "\\b") 'builtin)
(add-syntax-rule 'bash-mode "\\$\\{[^}]*\\}|\\$(?:[A-Za-z_][A-Za-z0-9_]*|[0-9@*#?$!_-])" 'variable)
(add-syntax-rule 'bash-mode "\\b([A-Za-z_][A-Za-z0-9_]*)=" 'variable 1)
(add-syntax-rule 'bash-mode "\\b([A-Za-z_][A-Za-z0-9_]*)[ \t]*\\([ \t]*\\)" 'function 1)
(add-syntax-rule 'bash-mode "<<-?|>>?|[|&;]+" 'keyword)
(set-syntax-pairs 'bash-mode "()[]{}")
(set-syntax-entry 'bash-mode "`" 'string)
(set-string-syntax 'bash-mode '(("'" "'")))
(set-comment-syntax 'bash-mode '(("#")))
(put 'bash-mode 'tab-width 4)
(put 'bash-mode 'indent-function 'bash-indent-line)

(defun bash-add-heredoc-delimiter (delimiter)
  "Colour as a string a heredoc whose marker is DELIMITER, from <<DELIMITER, or
<<-DELIMITER, or the marker quoted, to the line holding DELIMITER alone.
DELIMITER is a word, without white space or quotes."
  (when (or (string= delimiter "") (string-match-p "[ \t\r\n'\"]" delimiter))
    (error "A heredoc delimiter must be a nonempty word without whitespace or quotes"))
  (unless (member delimiter (get 'bash-mode 'heredoc-delimiters))
    (dolist (dash '("" "-"))
      (dolist (quote '("" "'" "\""))
        (add-syntax-region 'bash-mode
          (concat "<<" dash "[ \t]*" (regexp-opt (list (concat quote delimiter quote))) "([ \t;|&()]|$)")
          (concat "^" (if (string= dash "-") "\t*" "") (regexp-opt (list delimiter)) "$") 'string)))
    (put 'bash-mode 'heredoc-delimiters
         (append (get 'bash-mode 'heredoc-delimiters) (list delimiter))))
  t)

(dolist (delimiter '("EOF" "EOT" "END")) (bash-add-heredoc-delimiter delimiter))

(defun bash--lex-line (text quote)
  "Return (CODE QUOTE) for TEXT, masking literals and preserving offsets."
  (let ((at 0) (size (length text)) (code "") (parameter 0)
        (parameter-open "") (parameter-close "") (parameter-quote nil))
    (while (< at size)
      (let* ((ch (substring text at (+ at 1)))
             (next (if (< (+ at 1) size) (substring text (+ at 1) (+ at 2)) ""))
             (gap " "))
        (cond
         (quote
          (cond
           ((and (not (string= quote "'")) (string= ch "\\") (< (+ at 1) size))
            (setq at (+ at 1) gap "  "))
           ((string= ch (if (string= quote "$'") "'" quote)) (setq quote nil))))
         ((> parameter 0)
          (cond
           ((and (not (equal parameter-quote "'")) (string= ch "\\") (< (+ at 1) size))
            (setq at (+ at 1) gap "  "))
           (parameter-quote (when (string= ch parameter-quote) (setq parameter-quote nil)))
           ((member ch '("'" "\"" "`")) (setq parameter-quote ch))
           ((string= ch parameter-open) (setq parameter (+ parameter 1)))
           ((string= ch parameter-close) (setq parameter (- parameter 1)))))
         ((and (string= ch "$") (member next '("{" "(")))
          (setq parameter 1 parameter-open next parameter-close (if (string= next "{") "}" ")")
                at (+ at 1) gap "0 "))
         ((and (string= ch "$") (string= next "'"))
          (setq quote "$'" at (+ at 1) gap "0 "))
         ((member ch '("'" "\"" "`")) (setq quote ch gap "0"))
         ((and (string= ch "\\") (< (+ at 1) size))
          (setq at (+ at 1) gap "0 "))
         ((and (string= ch "#")
               (or (= at 0) (string-match-p "[ \t;|&()]" (substring text (- at 1) at))))
          (setq gap (make-string (- size at) " ") at (- size 1)))
         (t (setq gap ch)))
        (setq code (concat code gap) at (+ at 1))))
    (list code quote)))

(defun bash--column (stack)
  "Return the body column represented by STACK."
  (if stack
      (+ (nth 1 (car stack)) (* (indent-width)
          (if (eq (nth 2 (car stack)) 'case-body) 2 1)))
      0))

(defun bash--find (stack kind)
  "Return the tail of STACK headed by KIND, or nil."
  (while (and stack (not (eq (car (car stack)) kind))) (setq stack (cdr stack)))
  stack)

(defun bash--scan-code (code stack)
  "Return STACK after processing the unquoted shell tokens in CODE."
  (let ((at 0) (command t) (pattern nil))
    (while (string-match "[^ \t;|&(){}]+|;;&|;;|;&|&&|\\|\\||[;|&(){}]" code at)
      (let* ((token (match-string 0 code)) (begin (match-beginning 0))
             (end (match-end 0))
             (case-pattern (and stack (eq (nth 2 (car stack)) 'case-pattern))))
        (setq at end)
        (cond
         ((member token '(";;" ";&" ";;&"))
          (let ((tail (bash--find stack 'case)))
            (when tail (setq stack (cons (list 'case (nth 1 (car tail)) 'case-pattern) (cdr tail)))))
          (setq command t))
         ((member token '(";" "&&" "||" "|" "&")) (setq command t))
         ((and case-pattern (string= token ")"))
          (setq stack (cons (list 'case (nth 1 (car stack)) 'case-body) (cdr stack)) command t))
         ((and case-pattern (not (string= token "esac"))) (setq pattern t))
         ((and command (member token '("fi" "done" "esac")))
          (setq stack (cdr (bash--find stack (cond ((string= token "fi") 'if)
                                                    ((string= token "done") 'loop) (t 'case))))
                command nil))
         ((string= token "}") (setq stack (cdr (bash--find stack 'brace)) command nil))
         ((and (string= token ")") (not pattern))
          (when (and stack (eq (car (car stack)) 'subshell)) (setq stack (cdr stack)))
          (setq command nil))
         ((and command (member token '("else" "elif" "then" "do"))) (setq command t))
         ((and command (member token '("if" "for" "while" "until" "select" "case")))
          (setq stack (cons (list (cond ((string= token "if") 'if)
                                        ((string= token "case") 'case) (t 'loop))
                                  (bash--column stack)
                                  (if (string= token "case") 'case-pattern 'body)) stack)
                command nil))
         ((and (string= token "{")
               (or (= begin 0) (string-match-p "[ \t;()]" (substring code (- begin 1) begin)))
               (or (= end (length code)) (string-match-p "[ \t;]" (substring code end (+ end 1)))))
          (setq stack (cons (list 'brace (bash--column stack) 'body) stack) command t))
         ((and command (string= token "("))
          (setq stack (cons (list 'subshell (bash--column stack) 'body) stack) command t))
         ((member token '("(" ")")) nil)
         (t (setq command nil)))))
    stack))

(defun bash--heredocs (text code)
  "Return literal heredoc delimiters in TEXT whose operators are code."
  (let ((at 0) (out nil))
    (while (string-match "<<(-?)[ \t]*['\"]?([A-Za-z_][A-Za-z0-9_]*)['\"]?" text at)
      (let ((begin (match-beginning 0)) (end (match-end 0))
            (strip (string= (match-string 1 text) "-")) (word (match-string 2 text)))
        (when (and (or (= begin 0) (not (string= (substring text (- begin 1) begin) "<")))
                   (string= (substring code begin (+ begin 2)) "<<"))
          (setq out (append out (list (list word strip)))))
        (setq at end)))
    out))

(defun bash-indent-line ()
  "Return the column the line point is on should start at, in a shell script."
  (beginning-of-line)
  (let* ((target (point)) (existing (current-indentation))
         (cache (buffer-get 'bash-indent-cache))
         (valid (and cache (<= (nth 0 cache) target)
                     (equal (nth 6 cache) (indent-width))
                     (string= (nth 1 cache) (buffer-substring (point-min) (nth 0 cache)))))
         (stack (if valid (nth 2 cache))) (quote (if valid (nth 3 cache)))
         (heredocs (if valid (nth 4 cache))) (continued (if valid (nth 5 cache))))
    ;; Cache only a verified prefix. Edits anywhere in it, including undo,
    ;; invalidate it; adding a new line does not reparse the whole file.
    (goto-char (if valid (nth 0 cache) (point-min)))
    (while (< (point) target)
      (let ((text (current-line)))
        (if heredocs
            (when (string= (if (nth 1 (car heredocs))
                              (substring text (or (string-match-p "[^\t]" text) (length text))) text)
                          (car (car heredocs)))
              (setq heredocs (cdr heredocs)))
            (let* ((lexed (bash--lex-line text quote)) (code (car lexed)))
              (setq quote (nth 1 lexed) stack (bash--scan-code code stack)
                    heredocs (bash--heredocs text code)
                    continued (string-match-p "(\\\\|\\|\\||&&|\\|)[ \t]*$" code)))))
      (end-of-line)
      (forward-char))
    (buffer-put 'bash-indent-cache
                (list target (buffer-substring (point-min) target) stack quote heredocs continued (indent-width)))
    (let* ((code (prog-indent--trim (car (bash--lex-line (current-line) quote))))
           (kind (cond ((string-match-p "^(fi|else|elif|then)\\b" code) 'if)
                       ((string-match-p "^(done|do)\\b" code) 'loop)
                       ((string-match-p "^esac\\b" code) 'case)
                       ((string-prefix-p "}" code) 'brace)
                       ((and stack (eq (car (car stack)) 'subshell) (string-prefix-p ")" code)) 'subshell)))
           (closing (if kind (bash--find stack kind))))
      (cond ((or heredocs quote) existing)
            (closing (nth 1 (car closing)))
            (t (+ (bash--column stack) (if continued (indent-width) 0)))))))

(add-auto-mode "\\.(sh|bash)$" 'bash-mode)
(add-auto-mode "(^|[/\\\\])\\.?(bashrc|bash_profile|bash_login|bash_logout|profile)$" 'bash-mode)
(log "bash-mode loaded")
