;;; cc-mode --- colouring and indentation for C and C++ source.
;;;
;;; c-mode is for the files whose names end in .c and .h, cpp-mode for .cpp,
;;; .hpp, .cc, .hh, .cxx, .hxx, .c++, .h++, .C, .H, .ipp, .tpp, .ixx and
;;; .cppm. User manual: the `modes' page, and `indentation' for how they
;;; indent.

(make-mode 'c-mode)
(make-mode 'cpp-mode)

(put 'cpp-mode 'raw-string-styles nil)

(defconst c-keywords
  '("auto" "break" "case" "const" "continue" "default" "do" "else" "enum"
    "extern" "for" "goto" "if" "inline" "register" "restrict" "return"
    "sizeof" "static" "struct" "switch" "typedef" "union" "volatile" "while"
    "_Alignas" "_Alignof" "_Atomic" "_Generic" "_Noreturn" "_Static_assert"
    "_Thread_local" "alignas" "alignof" "constexpr" "static_assert"
    "thread_local" "typeof" "typeof_unqual")
  "C keywords, including the C11 and C23 additions.")

(defconst cpp-keywords
  (append (let ((shared nil))
            (dolist (word c-keywords)
              (unless (member word '("restrict" "_Alignas" "_Alignof" "_Atomic" "_Generic"
                                     "_Noreturn" "_Static_assert" "_Thread_local" "typeof" "typeof_unqual"))
                (setq shared (append shared (list word)))))
            shared)
    '("and" "and_eq" "asm" "bitand" "bitor" "catch" "class" "co_await"
      "co_return" "co_yield" "compl" "concept" "const_cast" "consteval"
      "constinit" "decltype" "delete" "dynamic_cast" "explicit" "export"
      "friend" "mutable" "namespace" "new" "noexcept" "not" "not_eq"
      "operator" "or" "or_eq" "private" "protected" "public"
      "reinterpret_cast" "requires" "static_cast" "template" "this" "throw"
      "try" "typeid" "typename" "using" "virtual" "xor" "xor_eq"))
  "C-family keywords plus C++ casts, alternative operators and modern keywords.")

(defconst c-constants '("true" "false" "NULL" "nullptr" "stdin" "stdout" "stderr")
  "Named values offered by the C-family modes.")

(defconst cpp-constants c-constants
  "Named values offered in C++ buffers.")

(defconst c-types
  '("void" "bool" "char" "short" "int" "long" "float" "double" "signed"
    "unsigned" "_Bool" "_Complex" "_Imaginary" "size_t" "ssize_t"
    "intptr_t" "uintptr_t" "ptrdiff_t" "int8_t" "int16_t" "int32_t" "int64_t"
    "uint8_t" "uint16_t" "uint32_t" "uint64_t" "wchar_t" "char16_t" "char32_t")
  "C primitive types and common standard-library typedefs.")

(defconst cpp-types
  (append (let ((shared nil))
            (dolist (word c-types)
              (unless (member word '("_Bool" "_Complex" "_Imaginary"))
                (setq shared (append shared (list word)))))
            shared)
          '("char8_t"))
  "Primitive and standard types offered in C++ buffers.")

(dolist (mode '(c-mode cpp-mode))
  (add-syntax-region mode "/\\*" "\\*/" 'comment nil nil)
  (add-syntax-rule mode "(?:u8|u|U|L)?'(\\\\.|[^'\\\\])+'" 'string)
  (add-syntax-region mode "\"" "\"" 'string "\\\\.")
  (add-syntax-rule mode "///.*|//!.*" 'doc-comment)
  (add-syntax-rule mode "//.*" 'comment)
  (add-syntax-rule mode "^[ \t]*#[ \t]*[A-Za-z_]+" 'preprocessor)
  (let ((words (if (eq mode 'c-mode) c-keywords cpp-keywords))
        (types (if (eq mode 'c-mode) c-types cpp-types)))
    (put mode 'keywords (append words (append c-constants types)))
    (add-syntax-rule mode (concat "\\b" (regexp-opt words) "\\b") 'keyword)
    (add-syntax-rule mode (concat "\\b" (regexp-opt c-constants) "\\b") 'builtin)
    (add-syntax-rule mode (concat "\\b" (regexp-opt types) "\\b") 'type))
  (add-syntax-rule mode "\\b[A-Za-z_][A-Za-z0-9_]*_t\\b" 'type)
  (add-syntax-rule mode "\\b[A-Z][A-Z0-9_]*\\b" 'builtin)
  (add-syntax-rule mode "\\b[A-Z][A-Za-z0-9_]*\\b" 'type)
  (add-syntax-rule mode "\\b([A-Za-z_][A-Za-z0-9_]*)[ \t]*\\(" 'function 1)
  (add-syntax-rule mode "\\b(?:0[xX][0-9a-fA-F']+(?:\\.[0-9a-fA-F']*)?(?:[pP][+-]?[0-9']+)?|0[bB][01']+|[0-9][0-9']*(?:\\.[0-9']*)?(?:[eE][+-]?[0-9']+)?)(?:[uUlLfFzZ]*|_[A-Za-z_][A-Za-z0-9_]*)\\b|\\.[0-9][0-9']*(?:[eE][+-]?[0-9']+)?[fFlL]?\\b" 'builtin)
  (set-syntax-pairs mode "()[]{}")
  (set-char-quote mode "'")
  (set-comment-syntax mode '(("//") ("/*" "*/" nil)))
  (put mode 'indent-char-literals t)
  (put mode 'tab-width 4)
  (put mode 'indent-function 'cc-indent-line))

(add-syntax-rule 'cpp-mode "\\[\\[[^]]*\\]\\]" 'attribute)

(define-face 'doc-comment "bright-cyan" nil '("italic"))
(define-face 'attribute "bright-yellow" nil)
(define-face 'preprocessor "bright-magenta" nil)

(defun cpp-add-raw-string-delimiter (delimiter)
  "Make R\"DELIMITER(...)DELIMITER\" a raw string in C++, with or without u8, u,
U or L before it, for the colouring and for the brackets alike. DELIMITER is
at most 16 characters, none a space, a parenthesis or a backslash."
  ;; Fixed delimiters are declared to both engines. The regexp API cannot
  ;; capture a tag at the opener and use it as a backreference at the closer.
  (when (or (> (length delimiter) 16) (string-match-p "[ ()\\\\\t\r\n]" delimiter))
    (error "A C++ raw-string delimiter must have at most 16 non-space characters, excluding () and backslash"))
  (let ((styles (get 'cpp-mode 'raw-string-styles)))
    (dolist (prefix '("" "u8" "u" "U" "L"))
      (let* ((open (concat prefix "R\"" delimiter "("))
             (close (concat ")" delimiter "\"")))
        (unless (member (list open close) styles)
          (add-syntax-region 'cpp-mode (regexp-opt (list open)) (regexp-opt (list close)) 'string)
          (setq styles (append styles (list (list open close)))))))
    (put 'cpp-mode 'raw-string-styles styles)
    (set-string-syntax 'cpp-mode styles))
  t)

(dolist (tag '("" "raw" "tag" "json" "xml" "sql" "glsl"))
  (cpp-add-raw-string-delimiter tag))

(defun cc--switch-brace-p (open)
  "Return t if the brace at OPEN starts a switch body."
  (goto-char open)
  (beginning-of-line)
  (let ((code (prog-indent--trim (prog-indent--code))))
    (when (string= code "{")
      (let ((previous (prog-indent--previous)))
        (when previous
          (setq code (nth 2 previous))
          (let ((outer (nth 4 (syntax-ppss (nth 0 previous)))))
            (while (and outer (string= (prog-indent--char outer) "("))
              (goto-char outer)
              (beginning-of-line)
              (setq code (prog-indent--trim (prog-indent--code))
                    outer (nth 4 (syntax-ppss))))))))
    (string-match-p "\\bswitch\\b" code)))

(defun cc--case-before (open start)
  "Return t if START follows a case label directly inside the brace at OPEN."
  (goto-char start)
  (let ((line (prog-indent--previous)) (found nil))
    (while (and line (> (nth 0 line) open) (not found))
      (when (and (equal (nth 4 (syntax-ppss (nth 0 line))) open)
                 (string-match-p "^(case\\b.*|default)[ \t]*:" (nth 2 line)))
        (setq found t))
      (unless found (setq line (prog-indent--previous))))
    found))

(defun cc--control-column (start open)
  "Return the nearest unbraced if header's column before START within OPEN."
  (goto-char start)
  (let ((line (prog-indent--previous)) (column nil))
    (while (and line (null column) (or (null open) (> (nth 0 line) open)))
      (when (and (equal (nth 4 (syntax-ppss (nth 0 line))) open)
                 (string-match-p "^(if|else[ \t]+if)\\b" (nth 2 line)))
        (setq column (nth 1 line)))
      (unless column (setq line (prog-indent--previous))))
    column))

(defun cc-indent-line ()
  "Return the column the line point is on should start at, in C or C++."
  (beginning-of-line)
  (let* ((start (point)) (existing (current-indentation)) (state (syntax-ppss))
         (code (prog-indent--trim (prog-indent--code))) (open (nth 4 state))
         (previous (progn (goto-char start) (prog-indent--previous)))
         (before (if previous (nth 2 previous) ""))
         (header before) (header-column (if previous (nth 1 previous) 0))
         (width (indent-width)))
    (when previous
      (let ((outer (nth 4 (syntax-ppss (nth 0 previous)))))
        (while (and outer (member (prog-indent--char outer) '("(" "[")))
          (goto-char outer)
          (setq header-column (current-indentation))
          (beginning-of-line)
          (setq header (prog-indent--trim (prog-indent--code))
                outer (nth 4 (syntax-ppss))))))
    (cond
     ((nth 2 state) existing)
     ((nth 3 state)
      (goto-char start)
      (setq code (prog-indent--trim (current-line)))
      (goto-char (nth 3 state))
      (if (string-match-p "^\\*" code) (+ (current-column) 1) existing))
     ((string-prefix-p "#" code) 0)
     ((and previous (string-match-p "^#.*\\\\[ \t]*$" before)) width)
     ((and open (member (prog-indent--char open) '("(" "[")))
      (prog-indent--under open (string-match-p "^[)\\]]" code)))
     (t
      (let* ((base (if open (progn (goto-char open) (current-indentation)) 0))
             (body (+ base (if open width 0)))
             (case-body (and open (cc--switch-brace-p open) (cc--case-before open start))))
        (cond
         ((string-prefix-p "}" code) base)
         ((and (eq (major-mode) 'cpp-mode) (string-match-p "^(public|protected|private)[ \t]*:" code)) base)
         ((string-match-p "^(case\\b.*|default)[ \t]*:" code) body)
         ((string-match-p "^else\\b" code) (or (cc--control-column start open) body))
         ((string-match-p "^[A-Za-z_][A-Za-z0-9_]*[ \t]*:[ \t]*$" code) base)
         ((and previous (not (string-match-p "[;{}][ \t]*$" before))
               (string-match-p "^(if|for|while|else|do)\\b" header))
          (+ header-column (if (string-prefix-p "{" code) 0 width)))
         ((and previous (string-match-p "[=,+*/&|?\\\\-][ \t]*$" before))
          (+ body width))
         (t (+ body (if case-body width 0)))))))))

(add-auto-mode "\\.(c|h)$" 'c-mode)
(add-auto-mode "\\.(cpp|hpp|cc|hh|cxx|hxx|c\\+\\+|h\\+\\+|C|H|ipp|tpp|ixx|cppm)$" 'cpp-mode)
(log "cc-mode loaded")
