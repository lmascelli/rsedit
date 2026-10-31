;;; occur --- listing every line that matches, and walking the list.
;;;
;;; M-s o lists the lines of this buffer that match a pattern, M-s g those of
;;; the files under a directory, and M-g n and M-g p walk the list from
;;; wherever you are. User manual: the `searching' page.
;;; Design: `doc/architettura-editor.typ, <mod-occur>`.

(require 'commands)
(require 'debug)

(make-mode 'occur-mode)

(defvar occur-window-height 12
  "How many rows the window showing a list of matches takes.")

;; ---------------------------------------------------------------------------
;; Reading the listing
;; ---------------------------------------------------------------------------

(defconst occur-header-lines 1
  "How many lines of a list come before its first match.")

(defun occur--index-here ()
  "The index of the entry on the cursor's line, or nil."
  ;; Counted, not parsed: a path with a colon reads two ways.
  (let ((index (- (line-number-at-point) occur-header-lines 1)))
    (if (and (>= index 0) (< index (or (results-count (current-buffer)) 0)))
        index
        nil)))

(defun occur--visit (index)
  "Visit entry INDEX using the shared results policy."
  (results-visit index (current-buffer)))

(defun occur--display (buffer)
  "Show BUFFER using the shared results policy and give it focus."
  (results-display buffer occur-window-height t)
  (goto-line 1)
  buffer)


(defcommand occur-goto () nil
  "Go to the match on the cursor's line, in the window the list was made from."
  (let ((index (occur--index-here)))
    (if (null index)
        (message "No result on this line")
        (occur--visit index))))



;; ---------------------------------------------------------------------------
;; Starting one
;; ---------------------------------------------------------------------------

(defcommand occur-show () nil
  "Preview the current result in another window, keeping focus here."
  (let ((index (occur--index-here)))
    (if (null index)
        (message "No result on this line")
        (results-visit index (current-buffer) t))))

(defcommand occur (pattern) ("sList lines matching: ")
  "List every line of this buffer containing PATTERN, in a buffer of its own."
  (let ((buffer (occur--scan pattern nil)))
    (if (null buffer)
        (message "Nothing to search")
        (occur--display buffer))))

(defcommand occur-regexp (pattern) ("sList lines matching regexp: ")
  "List every line of this buffer matching the regular expression PATTERN."
  (let ((buffer (occur--scan pattern t)))
    (if (null buffer)
        (message "Nothing to search")
        (occur--display buffer))))

(defcommand grep (pattern) ("sSearch the tree for: ")
  "List every line containing PATTERN in the files under the working
directory. The list fills in as the files are read."
  (let ((buffer (grep--scan pattern nil nil)))
    (if (null buffer)
        (message "Nothing to search")
        (occur--display buffer))))

(defcommand grep-regexp (pattern) ("sSearch the tree for regexp: ")
  "Like `grep', with PATTERN a regular expression."
  (let ((buffer (grep--scan pattern nil t)))
    (if (null buffer)
        (message "Nothing to search")
        (occur--display buffer))))

(defcommand grep-in (directory) ("fSearch which directory: ")
  "Like `grep', in DIRECTORY; the pattern is asked for next."
  (minibuffer-read "Search for:"
                   (lambda (pattern)
                     (unless (string= pattern "")
                       (let ((buffer (grep--scan pattern directory nil)))
                         (if buffer (occur--display buffer)))))
                   nil
                   nil))

;; ---------------------------------------------------------------------------
;; Keys and faces
;; ---------------------------------------------------------------------------

(defcommand grep-kill () nil
  "Stop the search filling this list; what it has found stays."
  (if (stop-worker (current-buffer))
      (message "Stopping the search")
      (message "No search is running in this buffer")))

(define-key nil "M-s o" 'occur)
(define-key nil "M-s g" 'grep)

(define-key 'occur-mode "<ret>" 'occur-goto)
(define-key 'occur-mode "o" 'occur-show)
(define-key 'occur-mode "n" 'next-line)
(define-key 'occur-mode "p" 'previous-line)
(define-key 'occur-mode "q" 'results-quit)
(define-key 'occur-mode "C-c C-k" 'grep-kill)

;; The header and the trailer, so the shape of the list reads at a glance.
(add-syntax-rule 'occur-mode "^--- .* ---$" 'comment)
;; What each line is in, up to the colon before the line number. A path may
;; contain a colon; the *last* colon-number-colon is the one the scan wrote.
(add-syntax-rule 'occur-mode "^(.*):[0-9]+: " 'type 1)

;; The two marks, as backgrounds, as a compilation has them. Naming the faces
;; turns the marks on: the walk marks nothing with faces nobody named.
(define-face 'results-here nil "bright-black")
(define-face 'results-target nil "bright-black")

(log "occur loaded")
(provide 'occur)
