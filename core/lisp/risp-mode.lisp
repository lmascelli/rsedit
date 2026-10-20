;;; risp-mode --- colouring and indentation for the editor's own Lisp.
;;;
;;; Colours the Lisp this editor evaluates, indents it, and tells the editor
;;; its brackets, strings and comments, in the files whose names end in .lisp
;;; and .risp. User manual: the `modes' page.
;;;
;;; The functions and macros it colours are not listed here: `all-functions'
;;; and `all-macros' are asked once, when this file loads, and again by
;;; `risp-refresh-vocabulary' -- not on every visit, which would compile a
;;; regexp over every name there is each time a file opens. Only the special
;;; forms are written out, because the evaluator has no binding for them to
;;; report. Completion asks the environment every time, and needs neither.

(make-mode 'risp-mode)

;; Kept in step with the evaluator by hand: these have no binding, so
;; `all-macros' and `all-functions' cannot report them.
(defconst risp-special-forms
  '("and" "backquote" "catch" "cond" "condition-case" "defconst" "defmacro"
    "defun" "dolist" "dotimes" "fiber" "if" "lambda" "let" "let*" "or" "prog1"
    "prog2" "progn" "quote" "setq" "spawn" "unless" "unwind-protect" "when"
    "while")
  "The forms the evaluator handles itself, which risp-mode colours as
keywords.")

(defconst risp-constants '("t" "nil")
  "The two values spelled as names, t and nil.")

;; ---------------------------------------------------------------------------
;; Regions and rules
;; ---------------------------------------------------------------------------
;;
;; Regions first, so their opening delimiters are seen before any rule that
;; might match inside them -- see the `rust-mode' header on the distinction.

;; The escape is why a string does not end at the quote in "say \"hi\"".
(add-syntax-region 'risp-mode "\"" "\"" 'string "\\\\.")

;; A line comment is a rule rather than a region: it always ends where the line
;; does. `;;;' and `;;' are the conventional weights and get no special face --
;; a comment is a comment, and three of them do not make it more so.
(add-syntax-rule 'risp-mode ";.*" 'comment)

;; A character literal: `?a', `?\n'. Before the symbol rules, or the `a' would
;; be coloured as part of a name.
(add-syntax-rule 'risp-mode "\\?\\\\?." 'builtin)

;; A keyword argument -- `:test', `:key' -- which this dialect uses in places
;; even though it has no keyword type.
(add-syntax-rule 'risp-mode ":[a-zA-Z][a-zA-Z0-9*/+<>=?!_-]*" 'builtin)

(add-syntax-rule 'risp-mode "\\b-?[0-9]+(\\.[0-9]+)?\\b" 'builtin)

;; ---------------------------------------------------------------------------
;; The vocabulary, asked for rather than listed
;; ---------------------------------------------------------------------------

;; The bounds are spelled out rather than written \b, because a Lisp name may
;; end in a character the regexp engine does not count as a word character --
;; 1+, car*, string< -- and \b after one of those matches in the wrong place.
;; What separates one symbol from the next is whitespace or a bracket.
(defun risp--word-pattern (words)
  "A regexp matching any of WORDS as a whole symbol, the symbol in group 2."
  (concat "(^|[ \t\n()\\[\\]'`,])(" (regexp-opt words) ")([ \t\n()\\[\\]]|$)"))

(defun risp-refresh-vocabulary ()
  "Colour the functions and macros defined since risp-mode loaded, asking the
interpreter for them again, and return t."
  ;; Group 2 in each: `risp--word-pattern' has to match the delimiters either
  ;; side to know where a symbol ends, and colouring them would paint over the
  ;; parenthesis that the sexp scanner and the reader both care about.
  (add-syntax-rule 'risp-mode (risp--word-pattern risp-special-forms) 'keyword 2)
  (add-syntax-rule 'risp-mode (risp--word-pattern risp-constants) 'builtin 2)
  (let ((macros (all-macros)))
    (if macros
        (add-syntax-rule 'risp-mode (risp--word-pattern macros) 'keyword 2)))
  (let ((functions (all-functions)))
    (if functions
        (add-syntax-rule 'risp-mode (risp--word-pattern functions) 'function 2)))
  ;; What `C-M-i' offers from `capf-mode-keywords'. The other completion
  ;; sources already ask the environment directly; this is the one that reads a
  ;; list, so it is given the same one the colours use.
  (put 'risp-mode 'keywords
       (append risp-special-forms (append risp-constants (all-macros))))
  t)

(risp-refresh-vocabulary)

;; A defun's name, coloured as a definition rather than as a call. After the
;; vocabulary rules so it wins over them: the name being defined is not yet a
;; function, and if it is being redefined it should still read as the
;; definition.
(add-syntax-rule 'risp-mode
                 "\\((defun|defmacro|defcommand|defconst)[ \t]+([^ \t()]+)"
                 'type 2)

;; ---------------------------------------------------------------------------
;; What the scanner needs
;; ---------------------------------------------------------------------------
;;
;; The grammar above says what text should look like; this says what it means.
;; Everything structural rests on it: `C-M-f', `C-M-k', the indenter, and
;; electric-pair's idea of what pairs with what.

(set-syntax-pairs 'risp-mode "()[]")
(set-comment-syntax 'risp-mode '((";")))

;; The rule that was waiting for this mode to exist -- see `indent.lisp', which
;; has shipped `lisp-indent-line' with nothing to attach it to.
(put 'risp-mode 'indent-function 'lisp-indent-line)
(put 'risp-mode 'tab-width 2)

(add-auto-mode "\\.lisp$" 'risp-mode)
(add-auto-mode "\\.risp$" 'risp-mode)

(log "risp-mode loaded")
