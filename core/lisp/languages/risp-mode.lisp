;;; risp-mode --- colouring and indentation for the editor's own Lisp.
;;;
;;; Colours the Lisp this editor evaluates, indents it, and tells the editor
;;; its brackets, strings and comments, in the files whose names end in .lisp
;;; and .risp. User manual: the `modes' page.
;;; Design: `doc/architettura-editor.typ, <mod-linguaggi>`.

(require 'commands)
(require 'debug)
(require 'indent)

(make-mode 'risp-mode)

;; Kept in step with the evaluator by hand: these have no binding to report.
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
;; Regions first; within a region nothing else applies.

;; The escape is why a string does not end at the quote in "say \"hi\"".
(add-syntax-region 'risp-mode "\"" "\"" 'string "\\\\.")

;; A line comment is a rule, since it ends with the line; `;;;' gets no face
;; of its own.
(add-syntax-rule 'risp-mode ";.*" 'comment)

;; A character literal: `?a', `?\n'. Before the symbol rules, or the `a' would
;; be coloured as part of a name.
(add-syntax-rule 'risp-mode "\\?\\\\?." 'builtin)

;; A keyword argument, `:test', used in places without a keyword type.
(add-syntax-rule 'risp-mode ":[a-zA-Z][a-zA-Z0-9*/+<>=?!_-]*" 'builtin)

(add-syntax-rule 'risp-mode "\\b-?[0-9]+(\\.[0-9]+)?\\b" 'builtin)

;; ---------------------------------------------------------------------------
;; The vocabulary, asked for rather than listed
;; ---------------------------------------------------------------------------

;; Bounds spelled out, not \b: a name may end in a character that is not a
;; word character, as `1+' and `string<' do.
(defun risp--word-pattern (words)
  "A regexp matching any of WORDS as a whole symbol, the symbol in group 2."
  (concat "(^|[ \t\n()\\[\\]'`,])(" (regexp-opt words) ")([ \t\n()\\[\\]]|$)"))

(defun risp--build-vocabulary ()
  "Make the rules that colour risp-mode's words, from what the interpreter knows."
  ;; Group 2, which leaves the delimiters out; a name for each rule, so that
  ;; building it again replaces it.
  (add-syntax-rule 'risp-mode (risp--word-pattern risp-special-forms) 'keyword 2
                   'risp-special-forms)
  (add-syntax-rule 'risp-mode (risp--word-pattern risp-constants) 'builtin 2
                   'risp-constants)
  (let ((macros (all-macros)))
    (if macros
        (add-syntax-rule 'risp-mode (risp--word-pattern macros) 'keyword 2
                         'risp-macros)))
  (let ((functions (all-functions)))
    (if functions
        (add-syntax-rule 'risp-mode (risp--word-pattern functions) 'function 2
                         'risp-functions)))
  ;; What `capf-mode-keywords' offers: the list the colours use.
  (put 'risp-mode 'keywords
       (append risp-special-forms (append risp-constants (all-macros)))))

(defcommand risp-refresh-vocabulary () nil
  "Colour the functions and macros defined since risp-mode loaded, asking the
interpreter for them again."
  (risp--build-vocabulary)
  (message "risp-mode colours %s functions and %s macros"
           (length (all-functions)) (length (all-macros))))

(risp--build-vocabulary)

;; A defun's name, as a definition: after the vocabulary, so that it wins.
(add-syntax-rule 'risp-mode
                 "\\((defun|defmacro|defcommand|defconst)[ \t]+([^ \t()]+)"
                 'type 2)

;; ---------------------------------------------------------------------------
;; What the scanner needs
;; ---------------------------------------------------------------------------
;;
;; What `C-M-f', the indenter and electric-pair read.

(set-syntax-pairs 'risp-mode "()[]")
(set-comment-syntax 'risp-mode '((";")))

;; The rule of indent.lisp.
(put 'risp-mode 'indent-function 'lisp-indent-line)
(put 'risp-mode 'tab-width 2)

(add-auto-mode "\\.lisp$" 'risp-mode)
(add-auto-mode "\\.risp$" 'risp-mode)

(log "risp-mode loaded")
(provide 'risp-mode)
