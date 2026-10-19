;;; buffer-list --- the buffers you have, in a buffer.
;;;
;;; C-x b switches to a buffer chosen by completion, and C-x C-b lists them all,
;;; one per line, with keys to switch to them and kill them. User manual: the
;;; `buffers' page.
;;;
;;; As in dired, the buffer is the state: every command reads the name back
;;; from the line on screen, by column, since a buffer name may contain spaces.

(make-mode 'buffer-list-mode)

(defconst buffer-list-buffer-name "*Buffer List*"
  "The buffer the listing is shown in, which lists itself too.")

(defconst buffer-list-name-column 2
  "The column the names start at, after the modified flag and a space.")

(defconst buffer-list-name-width 24
  "How many columns the names take. A longer name pushes the rest of its line
along rather than being cut.")

(defconst buffer-list-mode-width 20
  "How many columns the mode names take, by the same rule.")

;; ---------------------------------------------------------------------------
;; Drawing
;; ---------------------------------------------------------------------------

(defun buffer-list--pad (text width)
  "TEXT followed by enough spaces to reach WIDTH, or TEXT if it is longer."
  (if (>= (length text) width)
      (concat text " ")
      (concat text (make-string (- width (length text)) " "))))

(defun buffer-list--line (name)
  "The listing line for the buffer called NAME."
  (concat (if (buffer-modified-p name) "* " "  ")
          (buffer-list--pad name buffer-list-name-width)
          (buffer-list--pad (symbol-name (major-mode name))
                            buffer-list-mode-width)
          (let ((file (buffer-file-name name))) (if file file ""))))

(defun buffer-list--draw ()
  "Fill the listing buffer with one line per buffer."
  (set-buffer-read-only nil)
  (clear-buffer)
  (mapc (lambda (name) (insert (buffer-list--line name) "\n"))
        (all-buffer-names))
  (set-buffer-read-only t))

(defun buffer-list--name-here ()
  "The live buffer named on the cursor's line, or nil."
  ;; Read by column, not split on spaces: a buffer name may contain them.
  (let* ((line (current-line))
         (end (min (length line)
                   (+ buffer-list-name-column buffer-list-name-width))))
    (if (<= (length line) buffer-list-name-column)
        nil
        (let ((name (buffer-list--trim-right
                     (substring line buffer-list-name-column end))))
          ;; A name that is still live. One that is not means the listing is
          ;; stale -- the buffer was killed from somewhere else since it was
          ;; drawn -- and saying so beats acting on it.
          (if (member name (all-buffer-names)) name nil)))))

(defun buffer-list--trim-right (text)
  "TEXT without its trailing spaces."
  (let ((end (length text)))
    (while (and (> end 0) (string= " " (substring text (- end 1) end)))
      (setq end (- end 1)))
    (substring text 0 end)))

;; ---------------------------------------------------------------------------
;; The commands
;; ---------------------------------------------------------------------------

(defcommand buffer-list () nil
  "Show every buffer in a listing where keys switch to them and kill them."
  (buffer-create buffer-list-buffer-name 'buffer-list-mode)
  (switch-to-buffer buffer-list-buffer-name)
  (buffer-list--draw)
  (goto-line 1))

(defcommand buffer-list-refresh () nil
  "List the buffers again, keeping the cursor on the same line."
  (let ((here (line-number-at-point)))
    (buffer-list--draw)
    (goto-line here)))

(defcommand buffer-list-select () nil
  "Switch to the buffer on the cursor's line."
  (let ((name (buffer-list--name-here)))
    (if name
        (switch-to-buffer name)
        (message "No buffer on this line"))))

(defcommand buffer-list-kill () nil
  "Kill the buffer on the cursor's line, asking first when it has unsaved
changes."
  (let ((name (buffer-list--name-here)))
    (cond
     ((null name) (message "No buffer on this line"))
     ((string= name buffer-list-buffer-name)
      ;; Killing the listing from inside the listing would leave the cursor in
      ;; a buffer that no longer exists. `q' is how you close it.
      (message "Use q to put the listing away"))
     ((buffer-modified-p name)
      (minibuffer-read
       (format "%s has unsaved changes. Kill anyway? (yes/no)" name)
       (lambda (answer)
         (if (buffer-list--affirmative answer)
             (buffer-list--killed name)
             (message "Not killed")))
       nil nil))
     (t (buffer-list--killed name)))))

(defun buffer-list--affirmative (answer)
  "Whether ANSWER is a yes. Anything else, including empty, is a no."
  (or (string= "yes" (downcase answer)) (string= "y" (downcase answer))))

(defun buffer-list--killed (name)
  "Kill NAME and redraw, reporting what happened."
  (close-buffer name)
  ;; Back to the listing before redrawing: `close-buffer' may have moved the
  ;; focused window to whatever replaced the buffer it killed, and drawing into
  ;; whatever that turned out to be is how a listing ends up written over a
  ;; file.
  (switch-to-buffer buffer-list-buffer-name)
  (buffer-list-refresh)
  (message "Killed %s" name))

;; ---------------------------------------------------------------------------
;; C-x b
;; ---------------------------------------------------------------------------

(defun switch-to-buffer-candidates (input)
  "Buffer names matching INPUT, best first."
  (fuzzy-filter input (all-buffer-names)))

(defcommand switch-to-buffer-prompt () nil
  "Ask for a buffer by name, completing over those there are, and switch to it."
  (minibuffer-read "Switch to buffer:"
                   (lambda (name)
                     (if (member name (all-buffer-names))
                         (switch-to-buffer name)
                         (message "No buffer called %s" name)))
                   'switch-to-buffer-candidates
                   nil))

(define-key nil "C-x b" 'switch-to-buffer-prompt)
(define-key nil "C-x C-b" 'buffer-list)

(define-key 'buffer-list-mode "<ret>" 'buffer-list-select)
(define-key 'buffer-list-mode "d" 'buffer-list-kill)
(define-key 'buffer-list-mode "g" 'buffer-list-refresh)
(define-key 'buffer-list-mode "n" 'next-line)
(define-key 'buffer-list-mode "p" 'previous-line)
(define-key 'buffer-list-mode "q" '(close-buffer buffer-list-buffer-name))

(log "buffer-list loaded")
