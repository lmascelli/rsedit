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

(add-syntax-rule 'rust-mode
                 "\\b(fn|let|mut|const|static|pub|crate|mod|use|as|impl|trait|for|in|where|match|if|else|while|loop|break|continue|return|struct|enum|type|dyn|move|ref|self|super|unsafe|async|await)\\b"
                 'keyword)

(add-syntax-rule 'rust-mode "\\b(true|false|None|Some|Ok|Err)\\b" 'builtin)
(add-syntax-rule 'rust-mode "\\b(i8|i16|i32|i64|isize|u8|u16|u32|u64|usize|f32|f64|bool|char|str|String|Vec|Option|Result)\\b" 'type)

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

(add-auto-mode "\\.rs$" 'rust-mode)

(log "rust-mode loaded")
