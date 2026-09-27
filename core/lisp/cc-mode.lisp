;;; cc-mode --- colouring for C/C++ source.

(make-mode 'cpp-mode)

;; Regions first, so that their opening delimiters are seen before any rule
;; that might match inside them. Within a region nothing else applies: a
;; string is uniformly a string.

(add-syntax-region 'cpp-mode "/\\*" "\\*/" 'comment nil nil)

;; A character literal. It supports escapes like '\n', '\0', '\\', '\''.
;; Must be defined before strings open so single-quoted characters like '"' 
;; don't trigger string regions.
(add-syntax-rule 'cpp-mode "'(\\\\.|[^'\\\\])'" 'string)

;; Strings and raw strings.
;; The escape pattern allows strings with escaped quotes like "hello \"world\"".
(add-syntax-region 'cpp-mode "\"" "\"" 'string "\\\\.")

;; Line comments and Doxygen doc comments.
(add-syntax-rule 'cpp-mode "///.*" 'doc-comment)
(add-syntax-rule 'cpp-mode "//!.*" 'doc-comment)
(add-syntax-rule 'cpp-mode "//.*" 'comment)

;; Preprocessor directives (e.g., #include, #define, #ifdef).
(add-syntax-rule 'cpp-mode "^\\s*#\\s*[a-zA-Z_]+" 'preprocessor)

;; C++11 Attributes (e.g., [[nodiscard]], [[maybe_unused]]).
(add-syntax-rule 'cpp-mode "\\[\\[[^\\]]*\\]\\]" 'attribute)

;; The vocabulary, written once and used twice.
(defconst cpp-keywords
  '("auto" "break" "case" "catch" "class" "co_await" "co_return" "co_yield"
    "concept" "const" "consteval" "constexpr" "constinit" "continue"
    "decltype" "default" "delete" "do" "else" "enum" "explicit" "export"
    "extern" "for" "friend" "goto" "if" "inline" "mutable" "namespace"
    "new" "noexcept" "operator" "private" "protected" "public" "register"
    "requires" "return" "sizeof" "static" "static_assert" "struct" "switch"
    "template" "this" "thread_local" "throw" "try" "typedef" "typeid"
    "typename" "union" "using" "virtual" "volatile" "while")
  "Keywords reserved by C and C++.")

(defconst cpp-constants
  '("true" "false" "NULL" "nullptr" "stdin" "stdout" "stderr")
  "Values spelled as names.")

(defconst cpp-types
  '("void" "bool" "char" "short" "int" "long" "float" "double"
    "signed" "unsigned" "size_t" "ssize_t" "intptr_t" "uintptr_t"
    "ptrdiff_t" "int8_t" "int16_t" "int32_t" "int64_t"
    "uint8_t" "uint16_t" "uint32_t" "uint64_t"
    "char8_t" "char16_t" "char32_t" "wchar_t")
  "Built-in primitives and standard type definitions.")

;; All three are offered for completion.
(put 'cpp-mode 'keywords
     (append cpp-keywords (append cpp-constants cpp-types)))

(add-syntax-rule 'cpp-mode (concat "\\b" (regexp-opt cpp-keywords) "\\b") 'keyword)
(add-syntax-rule 'cpp-mode (concat "\\b" (regexp-opt cpp-constants) "\\b") 'builtin)
(add-syntax-rule 'cpp-mode (concat "\\b" (regexp-opt cpp-types) "\\b") 'type)

;; Standard C/POSIX type convention naming (e.g., my_type_t).
(add-syntax-rule 'cpp-mode "\\b[a-zA-Z_][a-zA-Z0-9_]*_t\\b" 'type)

;; Capitalised identifiers by convention (e.g., class/struct names like MyClass).
(add-syntax-rule 'cpp-mode "\\b[A-Z][A-Za-z0-9_]*\\b" 'type)

;; Function call matching using a capture group (matching '(' without coloring it).
(add-syntax-rule 'cpp-mode "\\b([a-zA-Z_][a-zA-Z0-9_]*)\\s*\\(" 'function 1)

;; Numeric literals (including hex 0x, binary 0b, octal 0, floats, and C++14 single-quote separators).
(add-syntax-rule 'cpp-mode "\\b(0[xX][0-9a-fA-F_']+|0[bB][01_']+|[0-9][0-9_']*(\\.[0-9_']*)?)\\b" 'builtin)

;; Custom faces invented for this mode.
(set-face 'doc-comment "bright-cyan" nil '("italic"))
(set-face 'attribute "bright-yellow" nil)
(set-face 'preprocessor "bright-magenta" nil)

;; Syntax matching pairs.
(set-syntax-pairs 'cpp-mode "()[]{}")

;; Single quote quote character handling for scanner balance.
(set-char-quote 'cpp-mode "'")

;; C++11 raw string syntactic definitions (e.g., R"(...)" or R"#(...)#").
(set-string-syntax 'cpp-mode '(("R\"(" ")\"") ("R\"#(" ")\"#")))

;; Comment syntax definition (explicitly non-nesting for block comments).
(set-comment-syntax 'cpp-mode '(("//") ("/*" "*/" nil)))

;; Auto-mode associations for C and C++ source/header files.
(add-auto-mode "\\.(c|h|cpp|hpp|cc|cxx|hxx|C|H)$" 'cpp-mode)

(log "cpp-mode loaded")