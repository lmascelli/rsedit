;;; occur --- listing every line that matches, and walking the list.
;;;
;;; M-s o lists the lines of this buffer that match a pattern, M-s g those of
;;; the files under a directory, and M-g n and M-g p walk the list from
;;; wherever you are. User manual: the `searching' page.
;;;
;;; This module is only the view. `occur--scan' and `grep--scan' find the
;;; matches and attach them to the listing as a result set, which
;;; `results-visit' and `next-error' read; which window shows the listing, and
;;; what its keys do, is all that is decided here. The walking keys are built
;;; in, so that no producer of a list depends on this module.

(make-mode 'occur-mode)

(defvar occur-window-height 12
  "How many rows the window showing a list of matches takes.")

;; The window the lists are shown in, and the window the newest was made from,
;; where its matches open. Window ids; either may have been closed since.
(setq occur--window nil)
(setq occur--origin nil)

;; ---------------------------------------------------------------------------
;; Reading the listing
;; ---------------------------------------------------------------------------

(defconst occur-header-lines 1
  "How many lines of a list come before its first match.")

(defun occur--index-here ()
  "The index of the entry on the cursor's line, or nil."
  ;; Counted, not parsed: a path with a colon in it reads two ways, and the
  ;; entry already knows where it is.
  (let ((index (- (line-number-at-point) occur-header-lines 1)))
    (if (and (>= index 0) (< index (or (results-count) 0)))
        index
        nil)))

(defun occur--visit (index)
  "Go to entry INDEX of this list, in the window the list was made from;
return the entry, or nil."
  (let ((listing (current-buffer)))
    ;; Failing that window, the next one: never the list's own, which would
    ;; put the match over the list.
    (unless (and occur--origin
                 (not (= occur--origin (selected-window)))
                 (select-window occur--origin))
      (other-window 1))
    (results-visit index listing)))

(defcommand occur-goto () nil
  "Go to the match on the cursor's line, in the window the list was made from."
  (let ((index (occur--index-here)))
    (if (null index)
        (message "No result on this line")
        (occur--visit index))))

(defcommand occur-show () nil
  "Show the match on the cursor's line, and stay in the list."
  (let ((listing (current-buffer))
        (here (selected-window))
        (index (occur--index-here)))
    (if (null index)
        (message "No result on this line")
        (progn (occur--visit index)
               (select-window here)
               ;; With one window the match took the list's place.
               (switch-to-buffer listing)))))

;; ---------------------------------------------------------------------------
;; Starting one
;; ---------------------------------------------------------------------------

(defun occur--display (buffer)
  "Show BUFFER in the strip the lists are shown in, give it the focus, and
return it."
  (let ((from (selected-window)))
    ;; Made from the strip itself, a list keeps the window the last one was
    ;; made from.
    (unless (and occur--window (= from occur--window))
      (setq occur--origin from))
    ;; One strip for every list, so a second search does not stack another.
    (if (and occur--window (select-window occur--window))
        (switch-to-buffer buffer)
        (progn
          (setq occur--window
                (display-buffer-at-bottom buffer occur-window-height))
          (select-window occur--window)))
    (goto-line 1)
    buffer))

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
  "List every line containing PATTERN in the files under the directory the
editor was started in. The list fills in as the files are read."
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
(define-key 'occur-mode "q" 'delete-window)
(define-key 'occur-mode "C-c C-k" 'grep-kill)

;; The header and the trailer, so the shape of the list reads at a glance.
(add-syntax-rule 'occur-mode "^--- .* ---$" 'comment)
;; What each line is in, up to the colon before the line number. A path may
;; contain a colon; the *last* colon-number-colon is the one the scan wrote.
(add-syntax-rule 'occur-mode "^(.*):[0-9]+: " 'type 1)

;; The two marks, as a compilation has them. Backgrounds, because the text
;; under them is already coloured. Naming the faces is also what turns the
;; marks on: the built-in walk looks them up by name and marks nothing when
;; nothing has named them.
(set-face 'results-here nil "bright-black")
(set-face 'results-target nil "bright-black")

(log "occur loaded")
