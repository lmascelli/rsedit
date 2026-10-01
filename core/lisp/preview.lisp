;;; preview --- show what a command would insert, before it inserts it.
;;;
;;; `C-x r t' asks for a string and then replaces a block with it on every
;;; line. Until you have answered, there is nothing to look at but the prompt --
;;; and whether a replacement lines up is exactly the kind of thing you can see
;;; and cannot work out.
;;;
;;; This shows it: as you type into the prompt, the string appears in the buffer
;;; at every line of the block, in the face virtual text uses, and goes again
;;; when the prompt is answered or abandoned.
;;;
;;; # What makes it possible
;;;
;;; `make-virtual-text' -- text drawn in the window and not in the buffer. The
;;; buffer is not modified, so nothing lands in the undo history, nothing is
;;; marked unsaved, and abandoning the prompt leaves the file exactly as it was.
;;; Inserting the string for real and taking it back out again would do all
;;; three, and would be visible in the undo history for the rest of the session.
;;;
;;; # Why it is a module and not part of the command
;;;
;;; Because the mechanism should not know what it is for. `string-rectangle'
;;; computes and edits; `make-virtual-text' draws; neither has heard of the
;;; other, and this file is the only thing that has heard of both. A second
;;; command worth previewing is another few lines here rather than a change to
;;; either of them.
;;;
;;; # How it knows when to run
;;;
;;; `post-command-hook', the same seam the completion strip refreshes on. It
;;; runs after every command including each `self-insert-command' in the
;;; minibuffer, so "as you type" needs no new mechanism -- and
;;; `pending-command' says which command the prompt belongs to, so this previews
;;; the one it knows about and leaves every other prompt alone.

(defconst preview-minibuffer-name "*Minibuffer*"
  "The buffer a minibuffer prompt reads into.

Named here rather than taken from `completion.lisp', which names it too: a
module that read another module's constant would be a module that stopped
working when that one was not loaded, and every module here loads on its own.")

(defconst preview-category 'preview
  "The category this module's virtual text belongs to.

Its own, so that clearing the preview cannot take down a language server's
hints or anything else drawn in the same buffer.")

(defconst preview-commands '("string-rectangle")
  "The commands this module previews, by name.

A list because adding one is adding a name. Each needs a function below that
knows what that command would do -- there is no way to preview a command in
general, since the preview *is* the knowledge of what it is about to do.")

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
(set-face 'preview "bright-black" nil '("italic"))

(log "preview loaded")
