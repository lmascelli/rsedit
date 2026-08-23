;; Minibuffer: a small, always-available prompt for reading a single line
;; of input from the user, with optional Tab-completion.
;;
;; The public entry point is `minibuffer-read'. What actually renders and
;; drives the prompt is decided by *minibuffer-read-function*, which
;; names a function with the same signature as `minibuffer-read' itself.
;; It defaults to `default-minibuffer-prompt', a small floating window
;; docked to the last few lines of the frame. Anything wanting a fancier
;; minibuffer (a real popup completion list, fuzzy matching, ...) can just
;; (setq *minibuffer-read-function* 'my-own-prompt) -- every caller of
;; `minibuffer-read' picks it up automatically, with no changes of its
;; own.

(make-mode 'minibuffer-mode)

;; Function called with the final input string when the user presses Return.
(setq *minibuffer-on-confirm* nil)
;; Function called with the current input string to get a list of
;; completion candidates (strings), or nil to disable completion.
(setq *minibuffer-on-change* nil)
;; Function called with no arguments when the user presses Escape.
(setq *minibuffer-on-cancel* nil)
;; The buffer that was current before the minibuffer was opened.
(setq *minibuffer-previous-buffer* nil)
;; The candidate list from the most recent completion request, or nil.
(setq *minibuffer-completions* nil)
;; Which candidate in *minibuffer-completions* is currently shown.
(setq *minibuffer-completion-index* 0)

;; Names the function that actually implements `minibuffer-read'. Rebind
;; this to replace the built-in minibuffer with a custom implementation;
;; see `minibuffer-read's docstring for the required signature.
(setq *minibuffer-read-function* 'default-minibuffer-prompt)

(defun minibuffer-set-content (s)
  "Replace the minibuffer buffer's contents with S."
  (clear-buffer)
  (mapc 'self-insert (split-string s "")))

(defun minibuffer-confirm ()
  "Called when the user presses Return in the minibuffer."
  (let ((input (buffer-string))
        (on-confirm *minibuffer-on-confirm*))
    (close-buffer "*Minibuffer*")
    (if on-confirm (funcall on-confirm input))))

(defun minibuffer-cancel ()
  "Called when the user presses Escape in the minibuffer."
  (let ((on-cancel *minibuffer-on-cancel*))
    (close-buffer "*Minibuffer*")
    (if on-cancel (funcall on-cancel))))

(defun minibuffer-complete ()
  "Called when the user presses Tab in the minibuffer. The first press
after a change to the input computes completion candidates and shows the
first one; further presses (as long as the input hasn't changed since)
cycle through the rest."
  (let ((current (buffer-string)))
    (if (and *minibuffer-completions*
             (string= current (nth *minibuffer-completion-index* *minibuffer-completions*)))
        ;; Still showing one of the last candidates we offered -> advance
        ;; to the next one, wrapping around.
        (progn
          (setq *minibuffer-completion-index*
                (mod (+ *minibuffer-completion-index* 1) (length *minibuffer-completions*)))
          (minibuffer-set-content (nth *minibuffer-completion-index* *minibuffer-completions*)))
        ;; Either the very first Tab press, or the user typed something
        ;; since the last completion -> ask for a fresh candidate list.
        (if *minibuffer-on-change*
            (let ((candidates (funcall *minibuffer-on-change* current)))
              (setq *minibuffer-completions* candidates)
              (setq *minibuffer-completion-index* 0)
              (if candidates
                  (minibuffer-set-content (nth 0 candidates))))))))

(defun minibuffer-cleanup ()
  "Runs via after-close-hook once the minibuffer buffer closes, however
it closed (confirm, cancel, or otherwise)."
  (switch-to-buffer *minibuffer-previous-buffer*)
  (setq *minibuffer-on-confirm* nil)
  (setq *minibuffer-on-change* nil)
  (setq *minibuffer-on-cancel* nil)
  (setq *minibuffer-completions* nil)
  (setq *minibuffer-completion-index* 0))

(defun default-minibuffer-prompt (prompt on-confirm on-change on-cancel)
  "The built-in *minibuffer-read-function*: a floating window docked to
the bottom 3 lines of the frame."
  (setq *minibuffer-previous-buffer* (current-buffer))
  (setq *minibuffer-on-confirm* on-confirm)
  (setq *minibuffer-on-change* on-change)
  (setq *minibuffer-on-cancel* on-cancel)
  (setq *minibuffer-completions* nil)
  (setq *minibuffer-completion-index* 0)
  (make-floating-window "*Minibuffer*" 1 (- frame-height 4) (- frame-width 2) 3
                         prompt 'minibuffer-mode))

(defun minibuffer-read (prompt on-confirm on-change on-cancel)
  "Read a line of input from the user via a minibuffer prompt.
PROMPT is shown as the window's title. ON-CONFIRM is called with the
final input string when the user presses Return. ON-CHANGE, if non-nil,
is called with the current input string whenever the user presses Tab,
and must return a list of completion candidate strings; repeated Tab
presses cycle through them. ON-CANCEL, if non-nil, is called with no
arguments when the user presses Escape.
Which implementation actually runs is controlled by
*minibuffer-read-function* -- see its comment above."
  (funcall *minibuffer-read-function* prompt on-confirm on-change on-cancel))

(add-hook 'minibuffer-mode "after-close-hook" 'minibuffer-cleanup)
(add-to-list 'after-resize-hook (lambda (nw nh) (log "Resize event handled")))

(define-key 'minibuffer-mode "<Return>" 'minibuffer-confirm)
(define-key 'minibuffer-mode "<Escape>" 'minibuffer-cancel)
(define-key 'minibuffer-mode "<Tab>" 'minibuffer-complete)

(defun command-execute-prompt ()
  (minibuffer-read "Command" nil nil nil))

(define-key nil "M-x" 'command-execute-prompt)

(defun eval-expression-confirm (input)
  "Called when the user presses Return after `eval-expression-prompt' --
evaluate INPUT (a string) as a Lisp expression and log INPUT alongside its
result, so it can be inspected in the diagnostic log."
  (log (format "%s => %s" input (eval-string input))))

(defun eval-expression-prompt ()
  "Read a Lisp expression via the minibuffer (M-:) and evaluate it in the
running editor, logging the result. Useful for testing code interactively."
  (minibuffer-read "Eval:" 'eval-expression-confirm nil nil))

(define-key nil "M-:" 'eval-expression-prompt)

(log "End of the minibuffer.lisp")
