;;; preview --- what C-x r t would put in, shown while you type it.
;;;
;;; While the prompt of a command in `preview-commands' is open, what is typed
;;; is drawn in the buffer as virtual text, which changes nothing, and goes
;;; when the prompt closes. User manual: the `minibuffer' page.
;;;
;;; `string-rectangle' and `make-virtual-text' know nothing of each other; this
;;; module, run on `post-command-hook', is the only thing that knows both, and
;;; `pending-command' says which command the open prompt belongs to.

;; Named here as well as in completion.lisp: each module loads on its own.
(defconst preview-minibuffer-name "*Minibuffer*"
  "The buffer a prompt reads into.")

(defconst preview-category 'preview
  "The category of this module's virtual text, so that clearing it takes
nothing else with it.")

(defconst preview-commands '("string-rectangle")
  "The names of the commands previewed. Each needs a function here that knows
what it would do.")

;; Which buffer the preview was last drawn in, so it can be cleared from there
;; rather than from wherever the cursor has since gone.
(setq preview--buffer nil)
;; What was last previewed, so an unchanged prompt costs a comparison rather
;; than a redraw of every line.
(setq preview--showing nil)

(defun preview--clear ()
  "Take down whatever is being previewed."
  (if preview--buffer
      (progn
        ;; The buffer may have been killed since -- a preview is drawn over
        ;; whatever was in front of you, and nothing stops you closing it.
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
            ;; The column may be past the end of a short line, and
            ;; `goto-char' with the line's end is what that comes to -- the
            ;; preview then shows at the end of the line, which is where
            ;; `string-rectangle' would pad out to and put it.
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
              ;; Cleared first, every time: this draws the whole preview
              ;; rather than working out what changed, which for a handful of
              ;; lines is cheaper than being clever and cannot drift.
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
        ;; No prompt, or not one this module previews. Either way whatever was
        ;; being previewed is no longer being asked about.
        (preview--clear))))

;; Registered for every mode -- `add-hook' with nil -- because which major mode
;; the buffer happens to be in has nothing to do with whether a prompt is open
;; in front of it.
(add-hook nil "post-command-hook" 'preview--refresh)

;; What a preview looks like. Dim rather than coloured, and the same whatever
;; the mode: the one thing the reader must never have to wonder is whether what
;; they are looking at is in the file.
(define-face 'preview "bright-black" nil '("italic"))

(log "preview loaded")
