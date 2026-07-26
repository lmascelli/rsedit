(make-mode 'minibuffer-mode)

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
(define-key 'minibuffer-mode  "<ret>" 'minibuffer-submit)
(define-key 'minibuffer-mode "<esc>" 'minibuffer-cancel)
(define-key 'minibuffer-mode "C-m" 'open-config)

(defun read-from-minibuffer (prompt callback)
  "Display the minibuffer with PROMPT and run CALLBACK upon submission."
  (setq *minibuffer-callback* callback)
  ;; Assuming screen dimensions are available or hardcoded for now:
  ;; Example opens a 100x3 box at the bottom of a 100x30 terminal
  (make-floating-window "*minibuffer*" 0 27 100 3 prompt)
  ;; Ensure we are using the correct hooks/mode if needed
  )

(log "End of the minibuffer.lisp")
