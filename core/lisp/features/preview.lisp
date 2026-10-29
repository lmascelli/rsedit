;;; preview --- what C-x r t would put in, shown while you type it.
;;;
;;; While the prompt of a command in `preview-commands' is open, what is typed
;;; is drawn in the buffer as virtual text, which changes nothing, and goes
;;; when the prompt closes. User manual: the `minibuffer' page.
;;; Design: `doc/architettura-editor.typ, <mod-preview>`.

;; Named here as well as in completion.lisp: each module loads on its own.
(defconst preview-minibuffer-name "*Minibuffer*"
  "The buffer a prompt reads into.")

(defconst preview-category 'preview
  "The category of this module's virtual text, so that clearing it takes
nothing else with it.")

(defconst preview-commands '("string-rectangle")
  "The names of the commands previewed. Each needs a function here that knows
what it would do.")

;; Which buffer the preview was drawn in, to clear it there.
(setq preview--buffer nil)
;; What was last previewed: an unchanged prompt costs a comparison.
(setq preview--showing nil)

(defun preview--clear ()
  "Take down whatever is being previewed."
  (if preview--buffer
      (progn
        ;; The buffer may have been killed since.
        (if (member preview--buffer (all-buffer-names))
            (with-current-buffer preview--buffer
              (lambda () (clear-virtual-text preview-category))))
        (setq preview--buffer nil)
        (setq preview--showing nil))))

(defun preview--input ()
  "What has been typed into the prompt so far, or nil if there is no prompt."
  (if (member preview-minibuffer-name (all-buffer-names))
      (with-current-buffer preview-minibuffer-name (lambda () (buffer-string)))
      nil))

(defun preview--rectangle (text)
  "Show TEXT at the left edge of the marked block, on each of its lines."
  (let ((bounds (rectangle-bounds)))
    (if (null bounds)
        nil
        (let ((line (nth 0 bounds))
              (last (nth 1 bounds))
              (column (nth 2 bounds)))
          (while (<= line last)
            (goto-line line)
            (beginning-of-line)
            ;; Past the end of a short line, the preview shows at its end,
            ;; where `string-rectangle' would pad out to.
            (forward-char column)
            (make-virtual-text (point) text 'preview 0 preview-category)
            (setq line (+ line 1)))))))

(defun preview--draw (command text)
  "Preview COMMAND inserting TEXT, in the buffer the prompt was opened over."
  (let ((target *minibuffer-previous-buffer*))
    (if (null target)
        nil
        (progn
          (setq preview--buffer target)
          (setq preview--showing text)
          (with-current-buffer
              target
            (lambda ()
              ;; Cleared and drawn whole, every time: a few lines, and
              ;; nothing to drift.
              (clear-virtual-text preview-category)
              (if (and (equal command "string-rectangle") (not (string= text "")))
                  (preview--rectangle text))))))))

(defun preview--refresh ()
  "Keep the preview in step with the prompt. On `post-command-hook'."
  (let ((command (pending-command)))
    (if (and command (member command preview-commands))
        (let ((text (preview--input)))
          (if (and text (not (equal text preview--showing)))
              (preview--draw command text)))
        ;; No prompt, or not one previewed.
        (preview--clear))))

;; For every mode: a prompt can be open in front of any buffer.
(add-hook nil "post-command-hook" 'preview--refresh)

;; Dim and the same in every mode: plainly not in the file.
(define-face 'preview "bright-black" nil '("italic"))

(log "preview loaded")
(provide 'preview)
