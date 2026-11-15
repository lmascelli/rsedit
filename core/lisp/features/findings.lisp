;;; findings --- what producers have said about files, drawn and listed.
;;;
;;; `findings-mode', on in every buffer unless turned off, draws the findings
;;; about the buffer's file: the spans, a sign in the gutter, a count in the
;;; status line and the message at point. M-g f lists them for M-g n to walk.
;;; User manual: the `findings' page.
;;; Design: `doc/architettura-editor.typ, <mod-findings>`.

(require 'commands)
(require 'debug)

(make-mode 'findings-list-mode)

(defvar findings-window-height 12
  "How many rows the window showing a list of findings takes.")

(defconst findings-list-buffer-name "*Findings*"
  "The buffer a list of findings is shown in.")

(define-minor-mode findings-mode nil
  "Draw the findings about the current buffer's file -- each span in the face
of its severity, a sign in the gutter, the count in the status line, the
message at point: on, off, or the other way with no argument. On in every
buffer unless turned off.

Example:
  (findings-mode -1)")

(define-globalized-minor-mode global-findings-mode findings-mode nil
  "Turn `findings-mode' on in every buffer, or off in all of them.
The buffers to come follow; with no argument, the other way from how it is. On
unless turned off." t)

;; ---------------------------------------------------------------------------
;; A finding as a line of the list
;; ---------------------------------------------------------------------------

(defun findings--get (finding key)
  "The property KEY of FINDING."
  (nth 1 (memq key finding)))

(defun findings--name (file base)
  "FILE named from the directory BASE, when it is in it."
  (if (string-prefix-p base file) (substring file (length base)) file))

(defun findings--line (finding base)
  "FINDING as a line of the list, its file named from BASE."
  (let ((start (findings--get finding :start))
        (code (findings--get finding :code)))
    (concat (findings--name (findings--get finding :file) base)
            ":" (number-to-string (car start))
            ;; From 1, as a compiler writes it and `compile' reads it.
            ":" (number-to-string (+ (cdr start) 1))
            ": " (symbol-name (findings--get finding :severity))
            ": " (or (car (split-string (findings--get finding :message) "\n")) "")
            (if code (concat " [" code "]") "")
            " (" (symbol-name (findings--get finding :producer)) ")"
            (if (findings--get finding :stale) " (stale)" ""))))

;; ---------------------------------------------------------------------------
;; The list
;; ---------------------------------------------------------------------------

(defun findings--draw (buffer)
  "Fill BUFFER with the findings it lists, asked of the store again, and
attach them to it as the list `next-error' walks."
  (with-current-buffer
   buffer
   (lambda ()
     (let* ((what (buffer-get 'findings-what))
            (file-p (eq (car what) 'file))
            (path (cdr what))
            (found (if file-p (findings-for-file path) (findings-under path)))
            (base (if file-p (file-name-directory path) (file-name-as-directory path)))
            (here (line-number-at-point))
            (at 1)
            (entries nil))
       (set-buffer-read-only nil)
       (clear-buffer)
       (insert "--- " (number-to-string (length found))
               (if (= (length found) 1) " finding" " findings")
               (if file-p " about " " under ") path " ---\n")
       (dolist (finding found)
         (let ((start (findings--get finding :start)))
           (setq at (+ at 1))
           (insert (findings--line finding base) "\n")
           (setq entries
                 (cons (list "file" (findings--get finding :file)
                             (car start) (cdr start) 0 "" 0 0 at)
                       entries))))
       ;; Written by the list, not by you: nothing to save.
       (set-buffer-modified-p nil)
       (set-buffer-read-only t)
       (results-put "findings" path (reverse entries) buffer)
       (goto-line here)))))

(defun findings--show (what)
  "List the findings WHAT names, (file . FILE) or (under . DIRECTORY), and
show the list."
  (buffer-create findings-list-buffer-name 'findings-list-mode)
  (buffer-put 'findings-what what findings-list-buffer-name)
  ;; Asked of the store again before each step of M-g n: the list follows
  ;; what the producers say now.
  (buffer-put 'results-refresh 'findings--draw findings-list-buffer-name)
  (findings--draw findings-list-buffer-name)
  (results-display findings-list-buffer-name findings-window-height t)
  (goto-line 1))

(defcommand findings-list () nil
  "List the findings about the current buffer's file, for M-g n to walk."
  (let ((file (buffer-file-name)))
    (if (null file)
        (message "This buffer visits no file")
        (findings--show (cons 'file file)))))

(defcommand findings-list-under (directory) ("fList the findings under: ")
  "List the findings about the files in DIRECTORY and below it, for M-g n to
walk."
  (findings--show (cons 'under (expand-file-name directory))))

(defcommand findings-list-refresh () nil
  "Ask the store again for the findings this list shows."
  (findings--draw (current-buffer)))

(defun findings--index-here ()
  "The index of the entry on the cursor's line, or nil."
  ;; Counted, not parsed: a path with a colon reads two ways.
  (let ((index (- (line-number-at-point) 2)))
    (if (and (>= index 0) (< index (or (results-count (current-buffer)) 0)))
        index
        nil)))

(defcommand findings-list-goto () nil
  "Go to the finding on the cursor's line, in the window the list was made
from."
  (let ((index (findings--index-here)))
    (if (null index)
        (message "No finding on this line")
        (results-visit index (current-buffer)))))

(defcommand findings-list-show () nil
  "Show the finding on the cursor's line in another window, keeping focus
here."
  (let ((index (findings--index-here)))
    (if (null index)
        (message "No finding on this line")
        (results-visit index (current-buffer) t))))

;; ---------------------------------------------------------------------------
;; Keys and faces
;; ---------------------------------------------------------------------------

(define-key nil "M-g f" 'findings-list)

(define-key 'findings-list-mode "<ret>" 'findings-list-goto)
(define-key 'findings-list-mode "o" 'findings-list-show)
(define-key 'findings-list-mode "g" 'findings-list-refresh)
(define-key 'findings-list-mode "n" 'next-line)
(define-key 'findings-list-mode "p" 'previous-line)
(define-key 'findings-list-mode "q" 'results-quit)

(add-syntax-rule 'findings-list-mode "^--- .* ---$" 'comment)
(add-syntax-rule 'findings-list-mode ": (error):" 'finding-error-sign 1)
(add-syntax-rule 'findings-list-mode ": (warning):" 'finding-warning-sign 1)
(add-syntax-rule 'findings-list-mode ": (info):" 'finding-info-sign 1)
(add-syntax-rule 'findings-list-mode ": (hint):" 'finding-hint-sign 1)

;; The two marks of the walk, as occur and compile name them: naming the faces
;; turns the marks on.
(define-face 'results-here nil "bright-black")
(define-face 'results-target nil "bright-black")

(log "findings loaded")
(provide 'findings)
