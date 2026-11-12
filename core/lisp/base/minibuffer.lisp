;;; minibuffer --- the floating prompt, for who prefers it to the split at
;;; the bottom of the frame.
;;;
;;; The prompt itself is built in, so that M-x works whatever fails to load:
;;; `default-minibuffer-prompt', a window across the bottom of the frame, as
;;; in Emacs. The floating one is here, all of it, with its two settings, and
;;; is not used until `*minibuffer-read-function*' names it.
;;; User manual: the `minibuffer' page.
;;; Design: `doc/architettura-editor.typ, <mod-minibuffer>`.

(require 'debug)

;; ---------------------------------------------------------------------------
;; The floating prompt
;; ---------------------------------------------------------------------------
;;
;; What the editor used to open: a bordered window in the middle of the frame,
;; titled with the question. Everything a prompt needs is the editor's --
;; `minibuffer-confirm', `minibuffer-cancel', `minibuffer-complete', the
;; history and `minibuffer-cleanup' all read the variables set below, and
;; closing the buffer closes the window -- so only where it is drawn is here.

(defvar minibuffer-width 60
  "How many columns wide the floating prompt is, its border included. A frame
too narrow for it gets one as wide as it can hold.")

(defvar minibuffer-height 3
  "How many rows tall the floating prompt is, its border included: 3 is one
line of text between two of border.")

(defconst floating-minibuffer--buffer-name "*Minibuffer*"
  "The buffer every prompt is read in, the floating one's too.")

(defun floating-minibuffer--size (value fallback)
  "VALUE when it is a number of at least 1, FALLBACK otherwise: a bad
`minibuffer-width' or `minibuffer-height' must not stop the prompt opening."
  (if (and (numberp value) (>= value 1)) (floor value) fallback))

(defun floating-minibuffer--name (value)
  "VALUE, a symbol or a string, as a string; nil for nil or the empty one."
  (let ((name (cond ((null value) nil)
                    ((stringp value) value)
                    (t (symbol-name value)))))
    (if (and name (not (string= name ""))) name nil)))

(defun floating-minibuffer-prompt (prompt on-confirm on-change on-cancel
                                   &optional mode key)
  "Open a prompt in a bordered window floating in the middle of the frame,
titled PROMPT, `minibuffer-width' columns by `minibuffer-height' rows, or
what the frame can hold. The arguments are those of `minibuffer-read'. Put
it in `*minibuffer-read-function*' to have every prompt open this way."
  ;; One at a time, as the editor's own: a second would share the first's
  ;; buffer and take its answer.
  (if (member floating-minibuffer--buffer-name (all-buffer-names))
      (signal 'error
              '("Command attempted to use minibuffer while in minibuffer")))
  ;; The state the editor's primitives read, as `default-minibuffer-prompt'
  ;; sets it; `minibuffer-cleanup' clears it when the window closes.
  (setq *minibuffer-previous-buffer* (current-buffer))
  (setq *minibuffer-on-confirm* on-confirm)
  (setq *minibuffer-on-change* on-change)
  (setq *minibuffer-on-cancel* on-cancel)
  (setq *minibuffer-completions* nil)
  (setq *minibuffer-completions-for* nil)
  (setq *minibuffer-completion-index* 0)
  (setq *minibuffer-history-key*
        (or (floating-minibuffer--name key)
            (if (stringp prompt) prompt "")))
  ;; The echo area owns the bottom row, so the prompt is centred in the rows
  ;; above it, with a column of margin on each side.
  (let* ((rows (max 1 (- frame-height 1)))
         (width (max 1 (min (floating-minibuffer--size minibuffer-width 60)
                            (- frame-width 2))))
         (height (max 1 (min (floating-minibuffer--size minibuffer-height 3)
                             rows)))
         (x (floor (/ (max 0 (- frame-width width)) 2)))
         (y (floor (/ (max 0 (- rows height)) 2))))
    (make-floating-window floating-minibuffer--buffer-name x y width height
                          (if (stringp prompt) prompt nil)
                          (or (floating-minibuffer--name mode)
                              "minibuffer-mode")))
  t)

(defun toggle-floating-minibuffer ()
  "Switch every prompt between the minibuffer split at the bottom of the
frame, the editor's own, and the window floating in the middle of it."
  (if (eq *minibuffer-read-function* 'floating-minibuffer-prompt)
      (progn
        (setq *minibuffer-read-function* 'default-minibuffer-prompt)
        (message "Prompts open at the bottom of the frame"))
      (progn
        (setq *minibuffer-read-function* 'floating-minibuffer-prompt)
        (message "Prompts open in a floating window"))))

(register-command "toggle-floating-minibuffer" nil)

(log "End of the minibuffer.lisp")
(provide 'minibuffer)