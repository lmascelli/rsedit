(make-mode 'fundamental-mode)
(make-mode 'lisp-interactive-mode)

(define-key "C-q" 'quit)
(define-key "C-s" 'save-buffer)
(define-key "<ret>" 'insert-newline)
(define-key "<backspace>" 'delete-backward-char)

(defun open-config ()
  (find-file "~/.config/rsedit/init.lisp"))

;; Global variable storing the lambda/symbol to call when Enter is pressed
(setq *minibuffer-callback* nil)

(defun minibuffer-submit ()
  "Submit the contents of the minibuffer to the registered callback."
  (let ((input-text (buffer-string)))
    (close-floating-window)
    (clear-buffer)
    ;; If a callback is registered, execute it with the string input
    (if *minibuffer-callback*
        (funcall *minibuffer-callback* input-text)
      (message "No callback bound."))))

(defun minibuffer-cancel ()
  "Abort the minibuffer interaction."
  (close-floating-window)
  (clear-buffer)
  (setq *minibuffer-callback* nil)
  (message "Quit"))

;; Create the minibuffer key bindings
(define-key "<ret>" 'minibuffer-submit)
(define-key "<esc>" 'minibuffer-cancel)
(define-key "C-m" 'open-config)

;; In your mode registry, we register the major mode
(make-mode 'minibuffer-mode)

(defun read-from-minibuffer (prompt callback)
  "Display the minibuffer with PROMPT and run CALLBACK upon submission."
  (setq *minibuffer-callback* callback)
  ;; Assuming screen dimensions are available or hardcoded for now:
  ;; Example opens a 100x3 box at the bottom of a 100x30 terminal
  (make-floating-window "*minibuffer*" 0 27 100 3 prompt)
  ;; Ensure we are using the correct hooks/mode if needed
  )

(log (function-doc 'minibuffer-submit))
(log (function-doc 'read-from-minibuffer))
(log (function-doc 'minibuffer-cancel))
(log "End of the stdlib.lisp")
