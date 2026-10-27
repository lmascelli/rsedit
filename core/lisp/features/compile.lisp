;;; compile --- run a build, and jump to what it complained about.
;;;
;;; C-c c runs a command and shows its output in a strip along the bottom;
;;; M-g n and M-g p walk the places it names. User manual: the `compile' page.
;;;
;;; The running is `shell-command-start', unchanged: this module is the mode of
;;; its output buffer and the navigation. The state is in the buffer, as in
;;; dired: the first line, `$ cargo build', is the command `g' runs again. Only
;;; which buffer M-g n walks is kept outside, since that is asked from another
;;; buffer.

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

;; Which buffer `M-g n' walks. Asked from another buffer, so it cannot be read
;; off the screen the way the command can.
(setq compilation--buffer nil)

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
;; `next-error' and `previous-error' are built in, and they walk a *result
;; set* -- the same one a search attaches to its listing. A compilation has one
;; complication a search does not: its output is still arriving, so the list
;; grows while you are walking it. That is what `results-refresh' is for. The
;; function below is registered against the compilation buffer and called
;; before each step, so `M-g n' sees what the compiler has said since the last
;; time you asked.
;;
;; Rebuilding means parsing the buffer again, which is a parse per line of
;; output -- for a compilation, a few hundred lines. The alternative is
;; parsing on every line as it arrives, which is the same work done more often
;; and at a worse moment.

(defun compilation--entries ()
  "Every place this buffer names, as entries of a result set."
  ;; (KIND SOURCE LINE COLUMN) is all a complaint says, so the offset, the
  ;; text and the match are left empty; the ninth element is the line of the
  ;; output the place is on, for the walk's mark.
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
it names."
  (let ((name (shell-command-start command 'compilation-mode 'compile)))
    (if (null name)
        nil
        (progn
          (setq compilation--buffer name)
          (remove-overlays 'results-here)
          ;; The list starts empty and grows as the compiler talks. Attached
          ;; now so that `M-g n' has something to walk from the first
          ;; complaint, and given a refresher so that what it walks is what
          ;; has arrived rather than what had arrived when this was attached.
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
;; list was made last -- a compilation, a search, anything that attached one --
;; and binding them here would make every other producer of a list depend on
;; this module being loaded.

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

;; The two marks. Backgrounds rather than foregrounds: the text under them is
;; already coloured by the rules above, and a background says "you are here"
;; without arguing with what the text is.
(define-face 'results-here nil "bright-black")
(define-face 'results-target nil "bright-black")

(log "compile loaded")
(provide 'compile)
