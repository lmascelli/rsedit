;;; buffer-list --- the buffers you have, in a buffer.
;;;
;;; C-x b switches to a buffer chosen by completion, and C-x C-b lists them all,
;;; one per line, with keys to switch to them, and to mark them, as Emacs'
;;; Buffer Menu does, to be saved and killed together. User manual: the
;;; `buffers' page.
;;; Design: `doc/architettura-editor.typ, <mod-buffer-list>`.

(require 'commands)
(require 'debug)

(make-mode 'buffer-list-mode)

(defconst buffer-list-buffer-name "*Buffer List*"
  "The buffer the listing is shown in, which lists itself too.")

(defconst buffer-list-name-column 4
  "The column the names start at, after the mark, the save flag, the modified
flag and a space.")

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

;; ---------------------------------------------------------------------------
;; Marks
;; ---------------------------------------------------------------------------
;;
;; As in Emacs' Buffer Menu: the first column is > for a buffer marked for D
;; and S, or D for one flagged to be killed by x; the second is S for one
;; flagged to be saved by x; the third is * for one with unsaved changes. Kept
;; by name, so that the listing drawn again shows them where they were.

;; The marks, as (NAME . MARK), MARK > or D.
(setq buffer-list--marks nil)

;; The buffers flagged to be saved by x.
(setq buffer-list--saves nil)

(defun buffer-list--mark-of (name)
  "The mark in NAME's first column, > or D, or nil."
  (let ((found (assoc name buffer-list--marks)))
    (if found (cdr found) nil)))

(defun buffer-list--without (name pairs)
  "PAIRS, an association list, without NAME's entry."
  (let ((kept nil))
    (mapc (lambda (pair) (unless (equal (car pair) name) (setq kept (cons pair kept))))
          pairs)
    (reverse kept)))

(defun buffer-list--forget (name)
  "Take NAME's marks away, both columns."
  (setq buffer-list--marks (buffer-list--without name buffer-list--marks))
  (let ((kept nil))
    (mapc (lambda (other) (unless (equal other name) (setq kept (cons other kept))))
          buffer-list--saves)
    (setq buffer-list--saves (reverse kept))))

(defun buffer-list--line (name)
  "The listing line for the buffer called NAME."
  (concat (let ((mark (buffer-list--mark-of name))) (if mark mark " "))
          (if (member name buffer-list--saves) "S" " ")
          (if (buffer-modified-p name) "* " "  ")
          (buffer-list--pad name buffer-list-name-width)
          (buffer-list--pad (symbol-name (major-mode name))
                            buffer-list-mode-width)
          (let ((file (buffer-file-name name))) (if file file ""))))

(defun buffer-list--redraw-line (name)
  "Write point's line again, for the buffer NAME, keeping point on it."
  (set-buffer-read-only nil)
  (beginning-of-line)
  (let ((start (point)))
    (delete-region start (+ start (length (current-line))))
    (insert (buffer-list--line name))
    (goto-char start))
  (set-buffer-read-only t))

(defun buffer-list--draw ()
  "Fill the listing buffer with one line per buffer."
  ;; Marks on buffers that have gone are forgotten with them.
  (let ((live (all-buffer-names)))
    (mapc (lambda (pair) (unless (member (car pair) live) (buffer-list--forget (car pair))))
          buffer-list--marks)
    (mapc (lambda (name) (unless (member name live) (buffer-list--forget name)))
          buffer-list--saves))
  (set-buffer-read-only nil)
  (clear-buffer)
  (mapc (lambda (name) (insert (buffer-list--line name) "\n"))
        (all-buffer-names))
  (set-buffer-read-only t))

(defun buffer-list--name-here ()
  "The live buffer named on the cursor's line, or nil."
  ;; Matched against the live names, not split on spaces, since a name may
  ;; contain them, nor cut at the column's width, which a long name runs
  ;; past: the longest live name the text starts with, ending there or at a
  ;; blank. A name no longer live means a stale listing: said, not acted on.
  (let ((line (current-line)))
    (if (<= (length line) buffer-list-name-column)
        nil
        (let ((rest (substring line buffer-list-name-column))
              (found nil))
          (mapc (lambda (name)
                  (if (and (string-prefix-p name rest)
                           (or (= (length name) (length rest))
                               (string= (substring rest (length name) (+ (length name) 1)) " "))
                           (or (null found) (> (length name) (length found))))
                      (setq found name)))
                (all-buffer-names))
          found))))

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

(defun buffer-list--change-mark (change)
  "Call CHANGE with the name on point's line to change its marks, draw the
line again, and go to the next one; say so when the line names no buffer."
  (let ((name (buffer-list--name-here)))
    (if name
        (progn (funcall change name) (buffer-list--redraw-line name))
        (message "No buffer on this line"))
    (next-line)))

(defcommand buffer-list-mark () nil
  "Mark the buffer on the cursor's line with >, for D and S, and go to the
next line."
  (buffer-list--change-mark
   (lambda (name)
     (setq buffer-list--marks
           (append (buffer-list--without name buffer-list--marks)
                   (list (cons name ">")))))))

(defcommand buffer-list-flag-kill () nil
  "Flag the buffer on the cursor's line with D, to be killed by x, and go to
the next line."
  (buffer-list--change-mark
   (lambda (name)
     (setq buffer-list--marks
           (append (buffer-list--without name buffer-list--marks)
                   (list (cons name "D")))))))

(defcommand buffer-list-flag-save () nil
  "Flag the buffer on the cursor's line with S, to be saved by x, and go to
the next line. A buffer that visits no file cannot be saved, and is not
flagged."
  (buffer-list--change-mark
   (lambda (name)
     (if (buffer-file-name name)
         (unless (member name buffer-list--saves)
           (setq buffer-list--saves (append buffer-list--saves (list name))))
         (message "%s visits no file" name)))))

(defcommand buffer-list-unmark () nil
  "Take the marks off the buffer on the cursor's line, and go to the next one."
  (buffer-list--change-mark 'buffer-list--forget))

(defcommand buffer-list-unmark-backward () nil
  "Go to the line above and take the marks off its buffer."
  (previous-line)
  (let ((name (buffer-list--name-here)))
    (if name
        (progn (buffer-list--forget name) (buffer-list--redraw-line name)))))

(defcommand buffer-list-unmark-all () nil
  "Take every mark off the listing."
  (setq buffer-list--marks nil)
  (setq buffer-list--saves nil)
  (buffer-list-refresh)
  (message "All marks removed"))

(defun buffer-list--marked (mark)
  "The buffers marked with MARK, in the order they are listed."
  (let ((out nil))
    (mapc (lambda (name) (if (equal (buffer-list--mark-of name) mark) (setq out (cons name out))))
          (all-buffer-names))
    (reverse out)))

(defun buffer-list--targets ()
  "What D and S act on: the buffers marked with >, or else the one on the
cursor's line, as a list of names; nil when there is neither."
  (let ((marked (buffer-list--marked ">")))
    (cond
     (marked marked)
     ((buffer-list--name-here) (list (buffer-list--name-here)))
     (t nil))))

;; ---------------------------------------------------------------------------
;; Saving and killing
;; ---------------------------------------------------------------------------

(defun buffer-list--save (names)
  "Save each of NAMES that visits a file, and return how many were saved."
  (let ((saved 0))
    (mapc (lambda (name)
            (if (buffer-file-name name)
                (progn
                  (with-current-buffer name (lambda () (save-buffer)))
                  (setq saved (+ saved 1)))
                (message "%s visits no file" name)))
          names)
    saved))

(defun buffer-list--kill (names)
  "Kill NAMES, asking once first when some of them have unsaved changes: a
no keeps those, and kills the others."
  (let ((wanted nil) (modified nil))
    (mapc (lambda (name)
            (cond
             ;; Killed from inside, the listing would leave the cursor nowhere.
             ((string= name buffer-list-buffer-name) nil)
             ((buffer-modified-p name)
              (setq modified (cons name modified))
              (setq wanted (cons name wanted)))
             (t (setq wanted (cons name wanted)))))
          names)
    (setq wanted (reverse wanted))
    (setq modified (reverse modified))
    (cond
     ((and (member buffer-list-buffer-name names) (null wanted))
      (message "Use q to put the listing away"))
     ((null wanted) (message "No buffer on this line"))
     ((null modified) (buffer-list--killed wanted nil))
     (t (yes-or-no
         (if (= (length modified) 1)
             (format "%s has unsaved changes. Kill anyway?" (car modified))
             (format "%s buffers have unsaved changes. Kill them anyway?"
                     (length modified)))
         (lambda () (buffer-list--killed wanted nil))
         (lambda () (buffer-list--killed wanted modified)))))))

(defun buffer-list--killed (names kept)
  "Kill NAMES but those in KEPT, redraw, and say what happened."
  (let ((count 0))
    (mapc (lambda (name)
            (unless (member name kept)
              (if (member name (all-buffer-names)) (close-buffer name))
              (buffer-list--forget name)
              (setq count (+ count 1))))
          names)
    ;; Back to the listing first: `close-buffer' may have shown another buffer
    ;; in the window, which the redraw would write over.
    (switch-to-buffer buffer-list-buffer-name)
    (buffer-list-refresh)
    (cond
     (kept (message "Killed %s, kept %s with unsaved changes" count (length kept)))
     ((= count 1) (message "Killed %s" (car names)))
     (t (message "Killed %s buffers" count)))))

(defcommand buffer-list-kill () nil
  "Kill the buffers marked with >, or the one on the cursor's line, asking
once first when some have unsaved changes."
  (buffer-list--kill (buffer-list--targets)))

(defcommand buffer-list-save () nil
  "Save the buffers marked with >, or the one on the cursor's line."
  (let ((names (buffer-list--targets)))
    (if names
        (let ((saved (buffer-list--save names)))
          (buffer-list-refresh)
          (if (> saved 0) (message "Saved %s buffer%s" saved (if (= saved 1) "" "s"))))
        (message "No buffer on this line"))))

(defcommand buffer-list-execute () nil
  "Save the buffers flagged with S, then kill those flagged with D, asking
once first when some of those have unsaved changes."
  (let ((saves buffer-list--saves)
        (kills (buffer-list--marked "D")))
    (cond
     ((and (null saves) (null kills)) (message "No buffers flagged with S or D"))
     (t (progn
          (if saves (buffer-list--save saves))
          (setq buffer-list--saves nil)
          (if kills
              (buffer-list--kill kills)
              (buffer-list-refresh)))))))

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
(define-key 'buffer-list-mode "m" 'buffer-list-mark)
(define-key 'buffer-list-mode "d" 'buffer-list-flag-kill)
(define-key 'buffer-list-mode "k" 'buffer-list-flag-kill)
(define-key 'buffer-list-mode "s" 'buffer-list-flag-save)
(define-key 'buffer-list-mode "u" 'buffer-list-unmark)
(define-key 'buffer-list-mode "<backspace>" 'buffer-list-unmark-backward)
(define-key 'buffer-list-mode "U" 'buffer-list-unmark-all)
(define-key 'buffer-list-mode "x" 'buffer-list-execute)
(define-key 'buffer-list-mode "D" 'buffer-list-kill)
(define-key 'buffer-list-mode "S" 'buffer-list-save)
(define-key 'buffer-list-mode "g" 'buffer-list-refresh)
(define-key 'buffer-list-mode "n" 'next-line)
(define-key 'buffer-list-mode "p" 'previous-line)
(define-key 'buffer-list-mode "q" '(close-buffer buffer-list-buffer-name))

;; The marks, coloured as dired's are.
(define-face 'dired-marked "yellow" nil '("bold"))
(define-face 'dired-flagged "red" nil '("bold"))
(add-syntax-rule 'buffer-list-mode "^>.*$" 'dired-marked)
(add-syntax-rule 'buffer-list-mode "^D.*$" 'dired-flagged)

(log "buffer-list loaded")
(provide 'buffer-list)
