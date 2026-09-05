;; Convenience on top of the command system.
;;
;; Nothing here is required. The registry, argument collection and M-x are all
;; hardcoded in Rust (`core/src/commands.rs' and `core/src/primitives/commands.rs'),
;; because a command that cannot collect its arguments is not a command, and a
;; configuration file should not be able to take that away by failing to load.
;; What is left here is sugar and policy: a shorter way to define a command,
;; and a place to change how completion behaves.
;;
;; To define a command without this file:
;;   (defun greet (who) (message "hello %s" who))
;;   (register-command 'greet '("sGreet whom: "))

(defmacro defcommand (name params specs &rest body)
  "Define NAME as a function taking PARAMS, and register it as a command whose
arguments the editor collects according to SPECS.

SPECS is a list of Emacs-style argument codes, one per parameter: \"sPROMPT\"
reads a string, \"nPROMPT\" a number, \"bPROMPT\" a buffer name with completion,
\"fPROMPT\" a file name. Use nil for a command taking no arguments.

The function defined is an ordinary function -- calling it from Lisp is a plain
call and prompts for nothing. `register-command' checks SPECS against PARAMS,
so the two cannot drift apart.

Example:
  (defcommand greet (who) (\"sGreet whom: \")
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
  "Return t if S starts with PREFIX."
  (and (<= (length prefix) (length s))
       (string= prefix (substring s 0 (length prefix)))))

(log "End of the commands.lisp")
