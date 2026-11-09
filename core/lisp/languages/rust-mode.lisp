;;; rust-mode --- colouring and indentation for Rust source.
;;;
;;; Colours Rust, indents it, and tells the editor its brackets, strings,
;;; character literals and comments, in the files whose names end in .rs.
;;; User manual: the `modes' page, and `indentation' for how it indents; how a
;;; mode like this one is written, `writing-modes'.
;;; Design: `doc/architettura-editor.typ, <mod-linguaggi>`.

(require 'indent)

(make-mode 'rust-mode)

;; Regions first; within a region nothing else applies.

;; Block comments nest in Rust: /* /* */ */ is one comment, not two.
(add-syntax-region 'rust-mode "/\\*" "\\*/" 'comment nil t)

;; A character literal: a rule, which begins at the `'' before the `"' of `'"'
;; can open a string. Not a region, which a lifetime would open.
(add-syntax-rule 'rust-mode "'(\\\\.|[^'\\\\])'" 'string)

;; The escape is why a string does not end at the quote in "say \"hi\"".
(add-syntax-region 'rust-mode "\"" "\"" 'string "\\\\.")
(add-syntax-region 'rust-mode "r#\"" "\"#" 'string)

;; Line comments: a rule, since they end with the line; doc comments get a
;; face of this module's own.
(add-syntax-rule 'rust-mode "///.*" 'doc-comment)
(add-syntax-rule 'rust-mode "//!.*" 'doc-comment)
(add-syntax-rule 'rust-mode "//.*" 'comment)

;; Attributes, likewise a face of this module's own.
(add-syntax-rule 'rust-mode "#!?\\[[^]]*\\]" 'attribute)


;; The vocabulary, written once: completed from the keywords property, and
;; coloured through `regexp-opt'.
(defconst rust-keywords
  '("fn" "let" "mut" "const" "static" "pub" "crate" "mod" "use" "as" "impl"
    "trait" "for" "in" "where" "match" "if" "else" "while" "loop" "break"
    "continue" "return" "struct" "enum" "type" "dyn" "move" "ref" "self"
    "super" "unsafe" "async" "await")
  "The words Rust reserves, which rust-mode colours as keywords.")

(defconst rust-constants '("true" "false" "None" "Some" "Ok" "Err")
  "The values Rust spells as names, which rust-mode colours as builtins.")

(defconst rust-types
  '("i8" "i16" "i32" "i64" "isize" "u8" "u16" "u32" "u64" "usize" "f32" "f64"
    "bool" "char" "str" "String" "Vec" "Option" "Result")
  "The types rust-mode colours by name; any name that begins with a capital
is coloured as a type too.")

;; All three are offered for completion.
(put 'rust-mode 'keywords
                   (append rust-keywords (append rust-constants rust-types)))

(add-syntax-rule 'rust-mode (concat "\\b" (regexp-opt rust-keywords) "\\b") 'keyword)
(add-syntax-rule 'rust-mode (concat "\\b" (regexp-opt rust-constants) "\\b") 'builtin)
(add-syntax-rule 'rust-mode (concat "\\b" (regexp-opt rust-types) "\\b") 'type)

;; A capitalised word is a type by convention.
(add-syntax-rule 'rust-mode "\\b[A-Z][A-Za-z0-9_]*\\b" 'type)

;; A macro invocation, and a plain call: a capture group leaves the `(' out,
;; where a lookahead would not be.
(add-syntax-rule 'rust-mode "\\b([a-z_][a-z0-9_]*!)" 'builtin 1)
(add-syntax-rule 'rust-mode "\\b([a-z_][a-z0-9_]*)\\s*\\(" 'function 1)

(add-syntax-rule 'rust-mode "\\b[0-9][0-9_]*(\\.[0-9_]+)?\\b" 'builtin)

;; The two faces this module names.
(define-face 'doc-comment "bright-cyan" nil '("italic"))
(define-face 'attribute "bright-yellow" nil)

;; What the scanner needs: the grammar says how the text looks, this what it
;; is.
(set-syntax-pairs 'rust-mode "()[]{}")

;; `'a'' is a character and `'static' a lifetime: the quote closes or not.
(set-char-quote 'rust-mode "'")

;; Raw strings, declared again here as in the grammar: both forms, or `r"..."'
;; reads as `r' and an ordinary string.
(set-string-syntax 'rust-mode '(("r#\"" "\"#") ("br#\"" "\"#") ("r\"" "\"") ("br\"" "\"")))
(set-comment-syntax 'rust-mode '(("//") ("/*" "*/" t)))

;; ---------------------------------------------------------------------------
;; Indentation
;; ---------------------------------------------------------------------------
;;
;; One `indent-width' per block, and one more for a line that goes on with the
;; statement above it: a method chain, an operator at the end of a line or at
;; the start of the next. Inside parentheses and brackets, under the first
;; argument, as in C. A comma ends nothing in Rust's blocks -- a struct's
;; fields, a match's arms -- so it does not make the next line a continuation,
;; as it does in cc-mode's rule.

(defun rust--continues-p (code previous)
  "Whether the line whose code is CODE goes on with the statement the line
whose code is PREVIOUS is part of."
  (or (string-match-p "^(\\.[A-Za-z_]|&&|\\|\\||\\?)" code)
      (string-match-p "(=>|=|\\+|-|\\*|/|%|&&|\\|\\||\\^|<<|>>)$" previous)))

(defun rust--statement-indentation ()
  "Return the indentation of the line the statement on point's line begins
on, going back over the lines it continues. Point ends at that line's start."
  (beginning-of-line)
  (let ((column (current-indentation))
        (done nil))
    (while (not done)
      (let* ((here (point))
             (code (prog-indent--trim (prog-indent--code)))
             (previous (prog-indent--previous)))
        (if (and previous (rust--continues-p code (nth 2 previous)))
            (progn
              (goto-char (nth 0 previous))
              (setq column (nth 1 previous)))
            (progn
              (goto-char here)
              (setq done t)))))
    column))

(defun rust-indent-line ()
  "Return the column the line point is on should start at, in Rust."
  (beginning-of-line)
  (let* ((start (point))
         (existing (current-indentation))
         (state (syntax-ppss))
         (code (prog-indent--trim (prog-indent--code)))
         (open (nth 4 state))
         (width (indent-width)))
    (cond
     ;; In a string the leading whitespace is content.
     ((nth 2 state) existing)
     ;; In a block comment, a `*' under the one that opened it.
     ((nth 3 state)
      (goto-char start)
      (setq code (prog-indent--trim (current-line)))
      (goto-char (nth 3 state))
      (if (string-match-p "^\\*" code) (+ (current-column) 1) existing))
     ((and open (member (prog-indent--char open) '("(" "[")))
      (prog-indent--under open (string-match-p "^[)\\]]" code)))
     (t
      ;; A block's body is one width in from the statement that opens it,
      ;; however many lines that statement took.
      (let* ((base (if open (progn (goto-char open) (rust--statement-indentation)) 0))
             (previous (progn (goto-char start) (prog-indent--previous))))
        (cond
         ((string-prefix-p "}" code) base)
         ;; The line above is in this block and the statement goes on.
         ((and previous
               (> (nth 0 previous) (or open -1))
               (rust--continues-p code (nth 2 previous)))
          (goto-char (nth 0 previous))
          (+ (rust--statement-indentation) width))
         (open (+ base width))
         (t 0)))))))

(put 'rust-mode 'tab-width 4)
(put 'rust-mode 'indent-function 'rust-indent-line)
;; A brace typed first on its line puts it in its place at once.
(put 'rust-mode 'electric-indent-chars '("{" "}"))

(add-auto-mode "\\.rs$" 'rust-mode)

(log "rust-mode loaded")
(provide 'rust-mode)
