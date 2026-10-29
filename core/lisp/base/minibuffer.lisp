;;; minibuffer --- M-:, an expression evaluated at a prompt.
;;;
;;; The prompt itself is built in, so that M-x works whatever fails to load.
;;; User manual: the `minibuffer' page.
;;; Design: `doc/architettura-editor.typ, <mod-minibuffer>`.

(require 'debug)

(defun eval-expression-confirm (input)
  "Evaluate INPUT, a string, and show it in the echo area with its value, or
with the error when it fails."
  ;; At the top level, so that what INPUT defines outlives this function; and
  ;; with the failure caught, so that a bad expression is reported rather than
  ;; ending the confirm.
  (let ((outcome (eval-string-top-level input)))
    (if (car outcome)
        (message "%s => %s" input (nth 1 outcome))
        (message "%s !! %s" input (nth 1 outcome)))))

(defun eval-expression-prompt ()
  "Ask for a Lisp expression, and evaluate it in the running editor. M-:."
  (minibuffer-read "Eval:" 'eval-expression-confirm nil nil))

;; A `defun' and a `register-command', as in common-keymaps.lisp, so that
;; this file loads without commands.lisp.
(register-command "eval-expression-prompt" nil)

(define-key nil "M-:" 'eval-expression-prompt)

(log "End of the minibuffer.lisp")
(provide 'minibuffer)
