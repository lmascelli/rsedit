;;; risp-mode --- colouring and indentation for the editor's own Lisp.
;;;
;;; The dialect this editor evaluates. Not Emacs Lisp and not Scheme, so the
;;; interesting question is where the vocabulary comes from -- and the answer is
;;; that most of it is not written down here at all.
;;;
;;; # Asking the interpreter instead of listing it
;;;
;;; `rust-mode' writes out Rust's keywords because it has to: nothing in this
;;; editor knows what Rust is. Risp is different -- the interpreter is right
;;; there, and it can be asked. `all-macros' and `all-functions' return exactly
;;; what is defined *in this editor, now*, including anything you added in your
;;; own init.lisp. A hand-written list would be wrong about that on the day it
;;; was written and wronger every time a primitive landed.
;;;
;;; The one part that is written out is the special forms. They are not macros
;;; and not functions -- `if' and `let' are built into the evaluator and have no
;;; binding to enumerate -- so `all-macros' cannot report them and this list is
;;; the only way to know them. It is short, and it changes when the evaluator
;;; changes, which is the right coupling: it is a list of what the evaluator
;;; treats specially, kept beside a mode that colours them specially.
;;;
;;; # When it is built
;;;
;;; Once, when this file loads. A function you define afterwards will not be
;;; coloured until `risp-refresh-vocabulary' is run -- which is what that
;;; command is for. Rebuilding on every visit would mean recompiling a regexp
;;; over every name in the image each time a file is opened, and the drift it
;;; would save is the drift of your own session.
;;;
;;; Completion does not have this problem: `capf-symbols' asks the environment
;;; on every keystroke, so `C-M-i' is always current whatever the colours say.

(make-mode 'risp-mode)

(defconst risp-special-forms
  '("and" "backquote" "catch" "cond" "condition-case" "defconst" "defmacro"
    "defun" "dolist" "dotimes" "fiber" "if" "lambda" "let" "let*" "or" "prog1"
    "prog2" "progn" "quote" "setq" "spawn" "unless" "unwind-protect" "when"
    "while")
  "What the evaluator handles itself, rather than by looking up a binding.

The only part of this mode's vocabulary that has to be written down: these have
no binding, so `all-macros' and `all-functions' cannot report them. Kept in step
with `eval.rs' by hand, and short enough that this is not a burden.")

(defconst risp-constants '("t" "nil")
  "The two values spelled as names.")

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

(defun risp--word-pattern (words)
  "A regexp matching any of WORDS as a whole symbol.

The bounds are spelled out rather than using `\\b' because a Lisp name may end
in a character the regexp engine does not think is a word character -- `1+',
`car*', `string<' -- and `\\b' after one of those matches in the wrong place.
What separates one symbol from the next in a Lisp is whitespace or a bracket,
so that is what this says."
  (concat "(^|[ \t\n()\\[\\]'`,])(" (regexp-opt words) ")([ \t\n()\\[\\]]|$)"))

(defun risp-refresh-vocabulary ()
  "Rebuild the colouring from what the interpreter currently knows.

Worth running after defining functions in a running editor -- from
`eval-expression', or by loading a file -- since the patterns are otherwise
built once when this mode loads. Not needed for completion, which asks the
environment every time it is invoked."
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
