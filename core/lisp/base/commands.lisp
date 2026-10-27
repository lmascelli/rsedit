;; Convenience on top of the command system. User manual: the
;; `writing-commands' page.
;;
;; Nothing here is required. The registry, argument collection and M-x are all
;; hardcoded in Rust (`core/src/commands.rs' and `core/src/primitives/commands.rs'),
;; because a command that cannot collect its arguments is not a command, and a
;; configuration file should not be able to take that away by failing to load.
;; What is left here is sugar: a shorter way to define a command, two small
;; functions on strings, `save-match-data' and `with-eval-after-load'.
;;
;; To define a command without this file:
;;   (defun greet (who) (message "hello %s" who))
;;   (register-command 'greet '("sGreet whom: "))

(defmacro defcommand (name params specs &rest body)
  "Define NAME as a function of PARAMS that runs BODY, and make it a command
whose arguments the editor collects as SPECS says: a list with a code for each
parameter, or nil for none. \"sPROMPT\" asks for a string, \"nPROMPT\" a
number, \"bPROMPT\" the name of a buffer and \"fPROMPT\" that of a file; \"p\"
is the prefix argument as a count, \"P\" as it was typed, and \"r\" the
region, as two arguments. A string at the head of BODY documents the command.

Called from Lisp, NAME is a function like any other, and asks for nothing.

Example:
  (defcommand greet (who) (\"sGreet whom: \")
    \"Say hello to WHO.\"
    (message \"hello %s\" who))"
  ;; Note the shapes used here. `,(symbol-name name)' passes the command's name
  ;; as a string literal, and `(list ,@specs)' builds the spec list at run time
  ;; from string literals -- neither needs a quote. That is deliberate: the
  ;; reader converts a quoted form to *data* as it parses, so an unquote written
  ;; inside one (`',name') is already data by the time the backquote expander
  ;; sees it, and never gets expanded.
  `(progn
     (defun ,name ,params ,@body)
     (register-command ,(symbol-name name) (list ,@specs))))

(defun string-prefix-p (prefix s)
  "t if S begins with PREFIX, nil if it does not.

Example:
  (string-prefix-p \"*Shell\" (current-buffer))"
  (and (<= (length prefix) (length s))
       (string= prefix (substring s 0 (length prefix)))))

(defun string-suffix-p (suffix s)
  "t if S ends with SUFFIX, nil if it does not.

Example:
  (string-suffix-p \".rs\" (buffer-file-name))"
  ;; The length test states the precondition rather than earning its keep:
  ;; `substring' clamps an index out of range -- (substring "abc" -1) is "c"
  ;; and (substring "abc" 5) is "" -- so a SUFFIX longer than S would come out
  ;; nil anyway, by luck rather than by reasoning. The day `substring'
  ;; complains about a range that is not there, this should not be what
  ;; breaks.
  (and (<= (length suffix) (length s))
       (string= suffix (substring s (- (length s) (length suffix))))))

(defmacro save-match-data (&rest body)
  "Run BODY, then put the match data back the way it was.

For a function that matches on somebody else's behalf. `string-match' records
where its groups landed for `match-string' to read later, and a caller holding
a match it has not read yet loses it to any function in between that matches
too -- with no sign at the point it reads the wrong groups. Wrapping the inner
matching in this is the convention, in Emacs and here.

The match data is put back however BODY ends, an error included.

Example:
  (defun name-of (file)
    (save-match-data
      (when (string-match \"([^/]+)$\" file)
        (match-string 1 file))))"
  ;; No gensym in this Lisp, so the saved data lives under a name nobody's BODY
  ;; is going to use. BODY could only see it by naming it.
  `(let ((save-match-data--saved (match-data)))
     (unwind-protect
         (progn ,@body)
       (set-match-data save-match-data--saved))))

(defmacro with-eval-after-load (feature &rest body)
  "Run BODY once FEATURE is loaded: now, if it is already, or else when it is
provided. FEATURE is evaluated, so a symbol is quoted. `eval-after-load' is
the same with a function in place of BODY.

Example:
  (with-eval-after-load 'dired
    (setq dired-sort 'type))"
  ;; Not `(lambda () ...)': the empty list a backquote builds is nil, which
  ;; `lambda' does not take for a parameter list. It is called with none.
  `(eval-after-load ,feature (lambda (&rest with-eval-after-load--none) ,@body)))

(log "End of the commands.lisp")
(provide 'commands)
