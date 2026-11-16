;;; compile --- run a build, and jump to what it complained about.
;;;
;;; C-c c runs a command and shows its output in a strip along the bottom;
;;; M-g n and M-g p walk the places it names, which are also reported as the
;;; findings of `compile'. User manual: the `compile' page.
;;; Design: `doc/architettura-editor.typ, <mod-compile>`.

(require 'commands)
(require 'debug)
(require 'shell)

(make-mode 'compilation-mode)

(defvar compilation-window-height 12
  "How tall the window showing a compilation's output is, in rows.")

;; A list, because compilers do not agree, and adding one is adding a line.
(defvar compilation-patterns
  '(;; gcc, clang, rustc's main line, most linters: path:line:col:
    ("^([^ :][^:]*):([0-9]+):([0-9]+)" 1 2 3)
    ;; make, older tools, and anything that omits the column.
    ("^([^ :][^:]*):([0-9]+):" 1 2 nil)
    ;; rustc's second line, which is where the useful path usually is:
    ;;   --> src/main.rs:12:9
    ("-->[ \t]+([^ :]+):([0-9]+):([0-9]+)" 1 2 3)
    ;; A Python traceback.
    ("File \"([^\"]+)\", line ([0-9]+)" 1 2 nil))
  "The patterns a line of output names a place by: a list of (REGEXP
FILE-GROUP LINE-GROUP COLUMN-GROUP), the groups numbered as `match-string'
numbers them, COLUMN-GROUP nil for a format with no column. The first that
matches a line decides.")

;; What a line says of itself, apart from where: gcc and clang say it after
;; the place, rustc on the line before the place.
(defvar compilation-severity-patterns
  '(("(^|[ :])(fatal error|error)(\\[([A-Za-z0-9_-]+)\\])?: *(.*)" error 5 4)
    ("(^|[ :])warning(\\[([A-Za-z0-9_-]+)\\])?: *(.*)" warning 4 3)
    ("(^|[ :])note: *(.*)" info 2 nil)
    ("(^|[ :])help: *(.*)" hint 2 nil))
  "How a line of output says how bad it is: a list of (REGEXP SEVERITY
MESSAGE-GROUP CODE-GROUP), SEVERITY one of `error', `warning', `info', `hint',
the groups nil when the format has none. The first that matches decides.")

(defvar compilation-default-severity 'error
  "The severity of a place that no line before it, nor its own, says.")

;; Which buffer `M-g n' walks. Asked from another buffer, so it cannot be read
;; off the screen the way the command can.
(setq compilation--buffer nil)

;; How far the newest build's output has been reported: the position after
;; the last line read, nil before the first; the line before it when it named
;; no place; and the findings so far, by file.
(setq compilation--read nil)
(setq compilation--before nil)
(setq compilation--found nil)
;; At most this many lines a report: a burst of output is read over several,
;; rather than holding up the keys for all of it at once.
(setq compilation--report-lines 500)

;; ---------------------------------------------------------------------------
;; Reading a line as a location
;; ---------------------------------------------------------------------------

(defun compilation--group (line index)
  "Group INDEX of the last match against LINE, or nil."
  (if (null index) nil (match-string index line)))

(defun compilation--parse (line)
  "(FILE LINE COLUMN) for LINE, COLUMN maybe nil, or nil if it names no place."
  ;; The first pattern to match decides, so the more specific come first.
  (let ((found nil))
    (mapc (lambda (pattern)
            (if (null found)
                (if (string-match (nth 0 pattern) line)
                    (setq found
                          (list (compilation--group line (nth 1 pattern))
                                (compilation--group line (nth 2 pattern))
                                (compilation--group line (nth 3 pattern)))))))
          compilation-patterns)
    found))

(defun compilation--location-here ()
  "The place the cursor's line names, or nil; the command on the first line
names none."
  (if (and (= (line-number-at-point) 1) (compilation--command-here))
      nil
      (compilation--parse (current-line))))

;; ---------------------------------------------------------------------------
;; Going there
;; ---------------------------------------------------------------------------

(defun compilation--index-here ()
  "Which of this buffer's places the cursor's line names, from 0, or nil."
  (if (null (compilation--location-here))
      nil
      (let ((here (line-number-at-point))
            (line 1)
            (count 0))
        (while (< line here)
          (goto-line line)
          (if (compilation--location-here) (setq count (+ count 1)))
          (setq line (+ line 1)))
        (goto-line here)
        count)))

(defcommand compilation-goto () nil
  "Open this line's location using the shared results policy."
  (let ((index (compilation--index-here))
        (output (current-buffer)))
    (if (null index)
        (message "No file and line on this line")
        (progn
          (compilation--rebuild output)
          (results-visit index output)))))

;; ---------------------------------------------------------------------------
;; Walking the complaints
;; ---------------------------------------------------------------------------

(defun compilation--step (direction)
  "t after moving to the next line naming a place, DIRECTION 1 or -1, else nil."
  (let ((start (line-number-at-point))
        (limit (progn (end-of-buffer) (line-number-at-point)))
        (found nil))
    (goto-line start)
    (let ((line (+ start direction)))
      (while (and (null found) (>= line 1) (<= line limit))
        (goto-line line)
        (if (compilation--location-here)
            (setq found t)
            (setq line (+ line direction)))))
    (if (null found) (goto-line start))
    found))

(defcommand compilation-next () nil
  "Move to the next line of the output that names a place."
  (if (not (compilation--step 1)) (message "No more")))

(defcommand compilation-previous () nil
  "Move to the previous line of the output that names a place."
  (if (not (compilation--step -1)) (message "No earlier one")))

;; ---------------------------------------------------------------------------
;; Joining the list everything else is walked with
;; ---------------------------------------------------------------------------
;;
;; The built-in walk reads a result set; the output still arrives, so
;; `compilation--rebuild' is its `results-refresh', called before each step.

(defun compilation--entries ()
  "Every place this buffer names, as entries of a result set."
  ;; Offset, text and match left empty; the ninth element is the output's line,
  ;; for the walk's mark.
  (let ((line 1)
        (last (progn (end-of-buffer) (line-number-at-point)))
        (entries nil))
    (while (<= line last)
      (goto-line line)
      (let ((location (compilation--location-here)))
        (if location
            (setq entries
                  (cons (list "file"
                              (nth 0 location)
                              (if (nth 1 location)
                                  (string-to-number (nth 1 location))
                                  1)
                              (if (nth 2 location)
                                  (- (string-to-number (nth 2 location)) 1)
                                  0)
                              0 "" 0 0
                              line)
                        entries))))
      (setq line (+ line 1)))
    (reverse entries)))

(defun compilation--rebuild (buffer)
  "Attach the places BUFFER names to it, as its result set."
  (with-current-buffer
   buffer
   (lambda ()
     (let ((here (point)))
       (results-put (or (compilation--command-here) "compilation")
                    buffer
                    (compilation--entries)
                    buffer)
       (goto-char here)))))

;; ---------------------------------------------------------------------------
;; Reporting the places as findings
;; ---------------------------------------------------------------------------
;;
;; The output only grows, so each report reads the lines that have come whole
;; since the last, and sets again only the files they name.

(defun compilation--severity (line)
  "(SEVERITY MESSAGE CODE) for what LINE says of how bad it is, or nil."
  (let ((found nil))
    (mapc (lambda (pattern)
            (if (and (null found) (string-match (nth 0 pattern) line))
                (setq found
                      (list (nth 1 pattern)
                            (compilation--group line (nth 2 pattern))
                            (compilation--group line (nth 3 pattern))))))
          compilation-severity-patterns)
    found))

(defun compilation--trim (text)
  "TEXT without the blanks around it."
  (if (string-match "^[ \t]*(.*?)[ \t\r]*$" text) (match-string 1 text) text))

(defun compilation--finding (location text before)
  "The finding the line TEXT naming LOCATION makes, BEFORE the line above."
  (let ((said (or (compilation--severity text)
                  (and before (compilation--severity before))))
        (line (if (nth 1 location) (string-to-number (nth 1 location)) 1))
        (column (if (nth 2 location) (string-to-number (nth 2 location)) 1)))
    (list :start (cons (max 1 line) (max 0 (- column 1)))
          :severity (if said (nth 0 said) compilation-default-severity)
          :message (or (nth 1 said) (compilation--trim text))
          :code (nth 2 said))))

(defun compilation--report (buffer)
  "t when BUFFER's whole lines are all reported, after reporting some more."
  (with-current-buffer
   buffer
   (lambda ()
     (let* ((here (point))
            (end (progn (goto-char (point-max)) (beginning-of-line) (point)))
            (from (or compilation--read (point-min)))
            (first (= from (point-min)))
            (lines (if (> end from)
                       (split-string (buffer-substring from end) "\n")
                       nil))
            ;; The text ends with a newline, so the last piece is empty.
            (left (min (- (length lines) 1) compilation--report-lines))
            (touched nil))
       (goto-char here)
       (while (> left 0)
         (let* ((text (car lines))
                (location (if (and first (compilation--command-here))
                              nil
                              (compilation--parse text))))
           (setq lines (cdr lines))
           (setq left (- left 1))
           (setq first nil)
           (setq from (+ from (length text) 1))
           (if location
               ;; Named from the directory the build ran in: a.c and ./a.c
               ;; are one file, not two whose findings replace each other.
               (let ((file (expand-file-name (nth 0 location))))
                 (setq compilation--found
                       (map-insert compilation--found file
                                   (cons (compilation--finding
                                          location text compilation--before)
                                         (map-elt compilation--found file))))
                 (if (not (member file touched))
                     (setq touched (cons file touched)))
                 (setq compilation--before nil))
               (setq compilation--before text))))
       (setq compilation--read from)
       (dolist (file touched)
         (finding-set 'compile file (reverse (map-elt compilation--found file))))
       (>= from end)))))

(defun compilation--ended-p (buffer)
  "t when the output in BUFFER ends with the line saying how its command ended."
  (with-current-buffer
   buffer
   (lambda ()
     (let ((here (point)))
       (goto-char (point-max))
       (if (and (= (current-column) 0) (> (line-number-at-point) 1))
           (previous-line))
       (let ((line (current-line)))
         (goto-char here)
         (if (string-match "^--- (exited|killed|could not be waited for)" line) t nil))))))

(defun compilation--follow (job)
  "Report more of the newest build; stop JOB once it is all reported and ended."
  (let ((buffer compilation--buffer))
    (if (or (null buffer) (not (member buffer (all-buffer-names))))
        (stop-worker job)
        ;; Whether it has ended asked first: the output still arrives, and
        ;; what is reported after must hold the line that said so.
        (let ((ended (compilation--ended-p buffer)))
          (if (and (compilation--report buffer) ended) (stop-worker job))))))

(defun compilation--report-anew ()
  "nil, after making the next report read the output from its start."
  (setq compilation--read nil)
  (setq compilation--before nil)
  (setq compilation--found (make-map))
  nil)

(defun compilation--watch ()
  "Start reporting the newest build's findings as its output comes."
  (compilation--report-anew)
  ;; The job only keeps time; the work is in its ON-PROGRESS, between commands,
  ;; where reading a buffer cannot move the point of a command running.
  (background-call 'compile-findings
                   (fiber (while t (yield '(sleep 250))))
                   'compilation--follow))

;; ---------------------------------------------------------------------------
;; Running it
;; ---------------------------------------------------------------------------

(defun compilation--command-here ()
  "The command this buffer's output came from, read off its first line, or nil."
  (let ((here (point)))
    (goto-line 1)
    (let ((line (current-line)))
      (goto-char here)
      (if (and (> (length line) 2) (string= "$ " (substring line 0 2)))
          (substring line 2)
          nil))))

(defun compilation--show (name)
  "Show NAME using the shared results policy, preserving focus."
  (results-display name compilation-window-height nil))

(defcommand compile (command) ("sCompile command: ")
  "Run COMMAND, showing its output as it comes; `next-error' walks the places
it names, and they replace the findings of `compile'."
  (let ((name (shell-command-start command 'compilation-mode 'compile)))
    (if (null name)
        nil
        (progn
          (setq compilation--buffer name)
          ;; A build starts over: what the last one said goes.
          (finding-clear 'compile)
          (compilation--watch)
          (remove-overlays 'results-here)
          ;; Attached empty now, with its refresher, so that `M-g n' walks
          ;; what has arrived by then.
          (results-put command name nil name)
          (buffer-put 'results-refresh 'compilation--rebuild name)
          (compilation--show name)))))

(defcommand compilation-recompile () nil
  "Run this output's command again."
  (let ((command (compilation--command-here)))
    (if command
        (compile command)
        (message "This buffer does not say what command it ran"))))

;; ---------------------------------------------------------------------------
;; Keys and faces
;; ---------------------------------------------------------------------------

(define-key nil "C-c c" 'compile)
;; `M-g n' and `M-g p' are bound in common-keymaps.lisp: they walk whatever
;; list was made last, by any producer.

(define-key 'compilation-mode "<ret>" 'compilation-goto)
(define-key 'compilation-mode "n" 'compilation-next)
(define-key 'compilation-mode "p" 'compilation-previous)
(define-key 'compilation-mode "g" 'compilation-recompile)
(define-key 'compilation-mode "q" 'results-quit)
;; A compilation is a shell command, and is stopped as one.
(define-key 'compilation-mode "C-c C-k" 'shell-command-kill)

;; The exit line, as in `shell-output-mode': a failure should be visible at a
;; glance rather than read.
(add-syntax-rule 'compilation-mode "^--- exited 0 ---$" 'comment)
(add-syntax-rule 'compilation-mode "^--- exited [1-9][0-9]* ---$" 'error)
(add-syntax-rule 'compilation-mode "^--- (killed|could not be waited for).*$" 'error)
(add-syntax-rule 'compilation-mode "^\\$ .*$" 'keyword)

;; The two marks, as backgrounds: the text under them keeps its colours.
(define-face 'results-here nil "bright-black")
(define-face 'results-target nil "bright-black")

(log "compile loaded")
(provide 'compile)
