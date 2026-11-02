;;; cmake-mode --- CMake colouring, bracket arguments and block indentation.
;;;
;;; For CMakeLists.txt and the files whose names end in .cmake. User manual:
;;; the `modes' page, and `indentation' for how it indents.
;;; Design: `doc/architettura-editor.typ, <mod-linguaggi>`.

(require 'indent)

(make-mode 'cmake-mode)

;; Bracket comments and bracket arguments, at the three lengths of = most
;; written: the closer repeats the opener's =, and a region has no
;; backreference to say so once. The comments first: they begin a column
;; earlier, at the #.
(add-syntax-region 'cmake-mode "#\\[\\[" "\\]\\]" 'comment)
(add-syntax-region 'cmake-mode "#\\[=\\[" "\\]=\\]" 'comment)
(add-syntax-region 'cmake-mode "#\\[==\\[" "\\]==\\]" 'comment)
(add-syntax-region 'cmake-mode "\\[\\[" "\\]\\]" 'string)
(add-syntax-region 'cmake-mode "\\[=\\[" "\\]=\\]" 'string)
(add-syntax-region 'cmake-mode "\\[==\\[" "\\]==\\]" 'string)
(add-syntax-region 'cmake-mode "\"" "\"" 'string "\\\\.")
(add-syntax-rule 'cmake-mode "#.*" 'comment)

(defconst cmake-keywords
  '("if" "elseif" "else" "endif" "foreach" "endforeach" "while" "endwhile"
    "function" "endfunction" "macro" "endmacro" "block" "endblock" "break"
    "continue" "return")
  "The CMake commands that make blocks or leave them, offered for completion.")

(defconst cmake-commands
  '("add_compile_definitions" "add_compile_options" "add_custom_command"
    "add_custom_target" "add_definitions" "add_dependencies" "add_executable"
    "add_library" "add_link_options" "add_subdirectory" "add_test"
    "cmake_minimum_required" "cmake_parse_arguments" "cmake_path"
    "cmake_policy" "configure_file" "enable_language" "enable_testing"
    "execute_process" "export" "file" "find_file" "find_library"
    "find_package" "find_path" "find_program" "get_filename_component"
    "get_property" "get_target_property" "include" "include_directories"
    "include_guard" "install" "link_directories" "link_libraries" "list"
    "math" "message" "option" "project" "set" "set_property"
    "set_target_properties" "string" "target_compile_definitions"
    "target_compile_features" "target_compile_options"
    "target_include_directories" "target_link_directories"
    "target_link_libraries" "target_link_options" "target_precompile_headers"
    "target_sources" "try_compile" "try_run" "unset")
  "The CMake commands most written, offered for completion.")

(defconst cmake-constants
  '("ON" "OFF" "TRUE" "FALSE" "YES" "NO" "IGNORE" "NOTFOUND")
  "The CMake values that are true or false, offered for completion.")

(defconst cmake-operators
  '("AND" "OR" "NOT" "COMMAND" "POLICY" "TARGET" "TEST" "DEFINED" "EXISTS"
    "IS_DIRECTORY" "IS_SYMLINK" "IS_ABSOLUTE" "IS_NEWER_THAN" "IS_READABLE"
    "IS_WRITABLE" "IS_EXECUTABLE" "MATCHES" "LESS" "GREATER" "EQUAL"
    "LESS_EQUAL" "GREATER_EQUAL" "STRLESS" "STRGREATER" "STREQUAL"
    "STRLESS_EQUAL" "STRGREATER_EQUAL" "VERSION_LESS" "VERSION_GREATER"
    "VERSION_EQUAL" "VERSION_LESS_EQUAL" "VERSION_GREATER_EQUAL" "PATH_EQUAL"
    "IN_LIST")
  "The operators of a CMake condition, offered for completion.")

(put 'cmake-mode 'keywords
     (append cmake-keywords (append cmake-commands (append cmake-constants cmake-operators))))

;; A command's name is matched in any case, as CMake reads it, and only where
;; a call follows: `set' is a command, SET in an argument list is a word.
(add-syntax-rule 'cmake-mode
                 (concat "(?i)\\b(" (regexp-opt cmake-keywords) ")[ \t]*\\(") 'keyword 1)
(add-syntax-rule 'cmake-mode
                 (concat "(?i)\\b(" (regexp-opt cmake-commands) ")[ \t]*\\(") 'builtin 1)
(add-syntax-rule 'cmake-mode "\\b([A-Za-z_][A-Za-z0-9_]*)[ \t]*\\(" 'function 1)
(add-syntax-rule 'cmake-mode (concat "\\b" (regexp-opt cmake-operators) "\\b") 'keyword)
(add-syntax-rule 'cmake-mode (concat "\\b" (regexp-opt cmake-constants) "\\b") 'builtin)
(add-syntax-rule 'cmake-mode "\\$(?:ENV|CACHE)?\\{[^}\n]*\\}" 'variable)
;; A generator expression is worked out when the build is generated, as the
;; preprocessor works out a directive before compiling.
(add-syntax-rule 'cmake-mode "\\$<[^>\n]*>" 'preprocessor)
(add-syntax-rule 'cmake-mode "\\b[0-9]+(?:\\.[0-9]+)*\\b" 'builtin)

(set-comment-syntax 'cmake-mode '(("#[[" "]]") ("#[=[" "]=]") ("#[==[" "]==]") ("#")))
(set-string-syntax 'cmake-mode '(("[[" "]]") ("[=[" "]=]") ("[==[" "]==]")))
(put 'cmake-mode 'tab-width 2)
(put 'cmake-mode 'indent-function 'cmake-indent-line)

;; ---------------------------------------------------------------------------
;; Indentation
;; ---------------------------------------------------------------------------

(defconst cmake--openers
  '("if" "elseif" "else" "foreach" "while" "function" "macro" "block")
  "The commands after which the next command goes one level in.")

(defconst cmake--closers
  '("elseif" "else" "endif" "endforeach" "endwhile" "endfunction" "endmacro"
    "endblock")
  "The commands that go one level out from the command before them.")

(defun cmake--head (code)
  "Return the name of the command CODE begins with, in lower case, or nil."
  (if (string-match "^([A-Za-z_][A-Za-z0-9_]*)" code)
      (downcase (match-string 1 code))
      nil))

(defun cmake--command-start (at)
  "Move to the start of the line where the command around AT begins."
  ;; Out of the strings and the argument lists AT is in, one at a time: a
  ;; command's arguments can run over several lines.
  (goto-char at)
  (let* ((state (syntax-ppss at))
         (outer (or (nth 2 state) (nth 4 state))))
    (while outer
      (goto-char outer)
      (beginning-of-line)
      (setq state (syntax-ppss (point))
            outer (or (nth 2 state) (nth 4 state)))))
  (beginning-of-line))

(defun cmake-indent-line ()
  "Return the column the line point is on should start at, in CMake."
  (beginning-of-line)
  (let* ((start (point)) (state (syntax-ppss))
         (code (prog-indent--trim (prog-indent--code)))
         (open (nth 4 state)))
    (cond
     ((or (nth 2 state) (nth 3 state)) (current-indentation))
     (open (prog-indent--under open (string-match-p "^\\)" code)))
     (t
      (goto-char start)
      (let ((previous (prog-indent--previous))
            (column 0))
        (when previous
          (cmake--command-start (nth 0 previous))
          (setq column (current-indentation))
          (when (member (cmake--head (prog-indent--trim (prog-indent--code)))
                        cmake--openers)
            (setq column (+ column (indent-width)))))
        (when (member (cmake--head code) cmake--closers)
          (setq column (- column (indent-width))))
        (max 0 column))))))

(add-auto-mode "(^|[/\\\\])CMakeLists\\.txt$" 'cmake-mode)
(add-auto-mode "\\.cmake$" 'cmake-mode)
(log "cmake-mode loaded")
(provide 'cmake-mode)
