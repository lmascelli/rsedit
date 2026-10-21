;;; toml-mode --- TOML strings, keys, tables and continuation indentation.

(make-mode 'toml-mode)
(add-syntax-region 'toml-mode "\"\"\"" "\"\"\"" 'string "\\\\.")
(add-syntax-region 'toml-mode "'''" "'''" 'string)
(add-syntax-region 'toml-mode "\"" "\"" 'string "\\\\.")
(add-syntax-region 'toml-mode "'" "'" 'string)
(add-syntax-rule 'toml-mode "#.*" 'comment)
;; Quoted keys keep their string face; a bare dotted key gets the variable face.
(add-syntax-rule 'toml-mode "([A-Za-z0-9_-]+(?:[ \t]*\\.[ \t]*[A-Za-z0-9_-]+)*)[ \t]*=" 'variable 1)
(add-syntax-rule 'toml-mode "^[ \t]*(\\[\\[?[^]#]+\\]\\]?)" 'type 1)
(defconst toml-constants '("true" "false" "inf" "nan")
  "TOML named values offered for completion.")
(put 'toml-mode 'keywords toml-constants)
(add-syntax-rule 'toml-mode (concat "\\b" (regexp-opt toml-constants) "\\b") 'builtin)
(add-syntax-rule 'toml-mode "\\b[0-9]{4}-[0-9]{2}-[0-9]{2}(?:[Tt ][0-9]{2}:[0-9]{2}:[0-9]{2}(?:\\.[0-9]+)?(?:[Zz]|[+-][0-9]{2}:[0-9]{2})?)?\\b|\\b[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\\.[0-9]+)?\\b" 'builtin)
(add-syntax-rule 'toml-mode "[+-]?\\b(?:0[xX][0-9a-fA-F_]+|0[oO][0-7_]+|0[bB][01_]+|[0-9][0-9_]*(?:\\.[0-9_]+)?(?:[eE][+-]?[0-9_]+)?)\\b" 'builtin)

(set-syntax-pairs 'toml-mode "[]{}")
(set-syntax-entry 'toml-mode "()" 'punctuation)
(set-comment-syntax 'toml-mode '(("#")))
;; Literal strings do not honour backslashes. Ordinary double quotes use
;; the scanner's escaped string class; triple forms need their own entries.
(set-string-syntax 'toml-mode '(("\"\"\"" "\"\"\"" t) ("'''" "'''") ("'" "'")))
(put 'toml-mode 'tab-width 2)
(put 'toml-mode 'indent-function 'toml-indent-line)

(defun toml-indent-line ()
  "(toml-indent-line): Return this TOML line's indentation; see `indentation'."
  (beginning-of-line)
  (let* ((state (syntax-ppss)) (existing (current-indentation))
         (code (prog-indent--trim (prog-indent--code))) (open (nth 4 state)))
    (cond
     ((nth 2 state) existing)
     (open (prog-indent--under open (string-match-p "^[\\]}]" code)))
     (t 0))))

(add-auto-mode "\\.toml$" 'toml-mode)
(add-auto-mode "(^|[/\\\\])Cargo\\.lock$" 'toml-mode)
(log "toml-mode loaded")
