;;; rust-mode --- colouring for Rust source.
;;;
;;; A language module: a `make-mode', a grammar, and an `add-auto-mode' so
;;; files of the right name open in it. Nothing here is special to Rust as far
;;; as the editor is concerned -- copy the shape for another language.
;;;
;;; Two things are worth knowing before writing one of these.
;;;
;;; The regexp engine has no lookahead, no lookbehind and no backreferences --
;;; that is how it stays linear. Where you would reach for `(?=...)', match the
;;; wider thing and face a capture group instead; where you would reach for
;;; `(?<!\\)', give the region an escape pattern.
;;;
;;; Anything that can cross a line is a *region*; anything that cannot is a
;;; *rule*. A line comment is a rule even though it looks like a region,
;;; because it always ends where the line does.

(make-mode 'rust-mode)

;; Regions first, so that their opening delimiters are seen before any rule
;; that might match inside them. Within a region nothing else applies: a
;; string is uniformly a string.

;; Block comments nest in Rust: /* /* */ */ is one comment, not two.
(add-syntax-region 'rust-mode "/\\*" "\\*/" 'comment nil t)

;; A character literal, and it has to come before the string region can open.
;; `'"'` contains a double quote: without this rule that quote opens a string
;; which never closes, and the rest of the file is coloured as one. The rule
;; wins because it starts one character earlier -- at the `'` -- and the scan
;; takes the leftmost match.
;;
;; `'` is deliberately *not* a string delimiter here. A lifetime is spelled the
;; same way, and `&'a str` would open a string that runs to the end of the
;; file. Requiring the closing quote is the whole of what tells the two apart.
(add-syntax-rule 'rust-mode "'(\\\\.|[^'\\\\])'" 'string)

;; The escape is why a string does not end at the quote in "say \"hi\"".
(add-syntax-region 'rust-mode "\"" "\"" 'string "\\\\.")
(add-syntax-region 'rust-mode "r#\"" "\"#" 'string)

;; Line comments: a rule, because they end with the line. Doc comments get
;; their own face -- which the editor has never heard of until this line names
;; it, and which `set-face' below then gives an appearance.
(add-syntax-rule 'rust-mode "///.*" 'doc-comment)
(add-syntax-rule 'rust-mode "//!.*" 'doc-comment)
(add-syntax-rule 'rust-mode "//.*" 'comment)

;; Attributes, likewise a face of this module's own.
(add-syntax-rule 'rust-mode "#!?\\[[^]]*\\]" 'attribute)


;; The vocabulary, written once and used twice.
;;
;; `set-mode-keywords' is what `capf-mode-keywords' offers when you complete in
;; a Rust buffer; `regexp-opt' turns the same list into the pattern that
;; colours it. Written as a regexp literal and a list side by side, the two
;; would agree today and disagree the first time someone added a keyword to one
;; of them -- and the failure would be a word that highlights but will not
;; complete, which nobody reports as a bug.
(defconst rust-keywords
  '("fn" "let" "mut" "const" "static" "pub" "crate" "mod" "use" "as" "impl"
    "trait" "for" "in" "where" "match" "if" "else" "while" "loop" "break"
    "continue" "return" "struct" "enum" "type" "dyn" "move" "ref" "self"
    "super" "unsafe" "async" "await")
  "The words Rust reserves.")

(defconst rust-constants '("true" "false" "None" "Some" "Ok" "Err")
  "Values spelled as names.")

(defconst rust-types
  '("i8" "i16" "i32" "i64" "isize" "u8" "u16" "u32" "u64" "usize" "f32" "f64"
    "bool" "char" "str" "String" "Vec" "Option" "Result")
  "The types worth knowing without being told.")

;; All three are offered for completion, because all three are things you type.
(put 'rust-mode 'keywords
                   (append rust-keywords (append rust-constants rust-types)))

(add-syntax-rule 'rust-mode (concat "\\b" (regexp-opt rust-keywords) "\\b") 'keyword)
(add-syntax-rule 'rust-mode (concat "\\b" (regexp-opt rust-constants) "\\b") 'builtin)
(add-syntax-rule 'rust-mode (concat "\\b" (regexp-opt rust-types) "\\b") 'type)

;; A capitalised word is a type by convention.
(add-syntax-rule 'rust-mode "\\b[A-Z][A-Za-z0-9_]*\\b" 'type)

;; A macro invocation, and then a plain call. Both need a capture group: the
;; thing that identifies them -- the `!' or the `(' -- must be matched but not
;; coloured, and there is no lookahead to match it without consuming it.
(add-syntax-rule 'rust-mode "\\b([a-z_][a-z0-9_]*!)" 'builtin 1)
(add-syntax-rule 'rust-mode "\\b([a-z_][a-z0-9_]*)\\s*\\(" 'function 1)

(add-syntax-rule 'rust-mode "\\b[0-9][0-9_]*(\\.[0-9_]+)?\\b" 'builtin)

;; The two faces this module invented. Every other face it used already had an
;; appearance; these did not exist at all until the rules above named them.
(set-face 'doc-comment "bright-cyan" nil '("italic"))
(set-face 'attribute "bright-yellow" nil)

;; What the scanner needs, as opposed to what the grammar above needs: the
;; grammar says what text should look like, this says what it means.
(set-syntax-pairs 'rust-mode "()[]{}")

;; `'a'` is one character, `'static` is a lifetime, and the scanner tells them
;; apart by whether the quote closes. Without this the `"` in `let c = '"';`
;; opens a string that never ends, and every brace after it stops counting --
;; which is how a closing brace three lines down became invisible to anything
;; asking whether the buffer balanced.
;;
;; The grammar above already colours char literals with a rule. A rule only
;; says what text should *look* like; this says what it *is*, which is what
;; `forward-sexp', the indenter and electric-pair read.
(set-char-quote 'rust-mode "'")

;; Raw strings. `r#"..."#' is one string however many quotes are inside it --
;; and without saying so here the scanner reads a raw string as a run of
;; ordinary strings with *code* between them, so any stray bracket in the
;; content opens a list that never closes. This editor's own source is the
;; example: `create_global_env' holds the default init.lisp in a raw string,
;; and the `"("' in one of its comments made everything after it scan wrong.
;;
;; Longest opener first is not needed -- the table sorts that out -- but the
;; two forms must both be here, or `r"..."' falls back to being a bare `r'
;; followed by an ordinary string.
;;
;; The grammar above declares the same thing again for colouring. They are
;; different questions -- what the text looks like, and what it means -- and a
;; mode may reasonably want a raw string coloured differently from a plain one.
;; Adding a string form means adding it in both places.
(set-string-syntax 'rust-mode '(("r#\"" "\"#") ("br#\"" "\"#") ("r\"" "\"") ("br\"" "\"")))
(set-comment-syntax 'rust-mode '(("//") ("/*" "*/" t)))

(add-auto-mode "\\.rs$" 'rust-mode)

(log "rust-mode loaded")
