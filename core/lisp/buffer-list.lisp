;;; buffer-list --- the buffers you have, in a buffer.
;;;
;;; Two ways to reach a buffer, and they answer different questions. `C-x b'
;;; is for when you know which one you want: type part of the name and the
;;; completion strip narrows to it. `C-x C-b' is for when you do not -- it
;;; shows the whole list, with enough about each one to recognise it by.
;;;
;;; # Where the state is
;;;
;;; Nowhere, the same way `dired' keeps none: the buffer is the state. Each
;;; line carries a name, and every command reads back what is on screen rather
;;; than consulting a table built when the listing was drawn. A table can
;;; disagree with the display -- after a buffer is killed elsewhere, after a
;;; refresh that failed halfway -- and when it disagrees, `d' kills something
;;; the user is not looking at.
;;;
;;; # The layout
;;;
;;;     * notes.txt          fundamental-mode  /home/user/notes.txt
;;;       *scratch*          fundamental-mode
;;;       core/src/main.rs   rust-mode         /home/user/p/core/src/main.rs
;;;
;;; A `*' in the first column for unsaved changes, then the name, the mode, and
;;; the file if there is one. Unlike `dired' -- whose header argues at length
;;; for one bare name per line -- the columns are worth their cost here: buffer
;;; names collide (`mod.rs' twice), and the path is what tells them apart.
;;;
;;; The name is still read by column rather than by splitting on spaces, since
;;; a buffer name may contain them.

(make-mode 'buffer-list-mode)

(defconst buffer-list-buffer-name "*Buffer List*"
  "The buffer the listing is shown in. It lists itself, which is honest: it is
a buffer, and pretending otherwise would mean a line that cannot be acted on.")

(defconst buffer-list-name-column 2
  "Where the name starts. Column 0 is the modified flag, column 1 a space.")

(defconst buffer-list-name-width 24
  "How wide the name column is. A longer name pushes the columns after it
along rather than being cut: a truncated buffer name is one you cannot switch
to by reading it, and the mode and path are only ever decoration.")

(defconst buffer-list-mode-width 20
  "How wide the mode column is, with the same rule about overflow.")

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
  "The buffer named on the cursor's line, or nil if the line names none.

Read by column rather than by splitting on whitespace: a buffer name may
contain spaces, and `*Buffer List*' itself does."
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
  "Show every buffer in a listing you can act on. Bound to C-x C-b.

RET switches to the buffer on the cursor's line, d kills it -- asking first if
it has unsaved changes -- g re-reads the list and q puts it away."
  (buffer-create buffer-list-buffer-name 'buffer-list-mode)
  (switch-to-buffer buffer-list-buffer-name)
  (buffer-list--draw)
  (goto-line 1))

(defcommand buffer-list-refresh () nil
  "Re-read the list of buffers, keeping the cursor on the same line.

How this module recovers from anything: every change ends here, and the buffer
is rebuilt from what is actually there rather than edited to match what was
expected."
  (let ((here (line-number-at-point)))
    (buffer-list--draw)
    (goto-line here)))

(defcommand buffer-list-select () nil
  "Switch to the buffer on the cursor's line. Bound to RET in buffer-list-mode."
  (let ((name (buffer-list--name-here)))
    (if name
        (switch-to-buffer name)
        (message "No buffer on this line"))))

(defcommand buffer-list-kill () nil
  "Kill the buffer on the cursor's line. Bound to d in buffer-list-mode.

A buffer with unsaved changes asks first. This is the one key in the module
that can lose work, and the listing is exactly where somebody is pressing keys
quickly down a column."
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
  "Ask which buffer to switch to, completing over the ones there are.

Bound to C-x b. `switch-to-buffer' is the primitive underneath and takes a name
already decided; this is the part that asks."
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
