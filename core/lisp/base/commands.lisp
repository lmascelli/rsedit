;;; commands --- a shorter way to define a command, and a few macros.
;;;
;;; `defcommand', `string-prefix-p', `string-suffix-p', `save-match-data' and
;;; `with-eval-after-load'. User manual: the `writing-commands' page.
;;; Design: `doc/architettura-editor.typ, <mod-commands>`.

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
  ;; The name as a string, the specs through `list': an unquote inside a quote,
  ;; `',name', is data before the backquote is expanded.
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
  ;; The length test is the precondition: that `substring' clamps a range out
  ;; of bounds is not relied on.
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
