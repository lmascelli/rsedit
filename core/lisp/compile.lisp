;;; compile --- run a build, and jump to what it complained about.
;;;
;;; `M-x compile' runs a command and shows its output. Return on a line that
;;; names a file and a line number opens that file, there, in a window beside
;;; the output. `M-g n' and `M-g p' walk the complaints without going back to
;;; the buffer at all.
;;;
;;; # What this module is, and is not
;;;
;;; It is a mode and some navigation. The running is `shell-command-start',
;;; unchanged: that already runs asynchronously, appends output as it arrives,
;;; writes an `--- exited N ---' trailer and keeps the renderer awake while it
;;; works. A second way of running processes would be a second thing to keep
;;; correct.
;;;
;;; # Where the state is
;;;
;;; In the buffer, as in `dired'. `shell-command-start' writes the command it
;;; ran as the first line -- `$ cargo build' -- so `g' reads it back off the
;;; screen rather than from a variable that can disagree with what is displayed.
;;; Two compilations in two buffers each re-run their own command, and neither
;;; needs a table mapping buffers to commands.
;;;
;;; The one thing kept outside is which buffer `M-g n' should walk, since that
;;; question is asked from a *different* buffer and so cannot be answered by
;;; reading the current one.

(make-mode 'compilation-mode)

(defconst compilation-window-height 12
  "How tall the window showing a compilation is.

A strip across the bottom, like `M-!'. The output is something you glance at
while working, and the file you jumped to is what deserves the room.")

;; What counts as a location, and which group is which.
;;
;; Each entry is (REGEXP FILE-GROUP LINE-GROUP COLUMN-GROUP), where the groups
;; are group numbers as `match-string' takes them -- 0 being the whole match, so
;; the first capture is 1. A nil column group means the format has no column.
;;
;; A list rather than one pattern because compilers do not agree, and a list is
;; how adding one is adding a line. What is here covers gcc, clang, most
;; linters, Python tracebacks and rustc.
(setq compilation-patterns
      '(;; gcc, clang, rustc's main line, most linters: path:line:col:
        ("^([^ :][^:]*):([0-9]+):([0-9]+)" 1 2 3)
        ;; make, older tools, and anything that omits the column.
        ("^([^ :][^:]*):([0-9]+):" 1 2 nil)
        ;; rustc's second line, which is where the useful path usually is:
        ;;   --> src/main.rs:12:9
        ("-->[ \t]+([^ :]+):([0-9]+):([0-9]+)" 1 2 3)
        ;; A Python traceback.
        ("File \"([^\"]+)\", line ([0-9]+)" 1 2 nil)))

(defconst compilation-mark-priority 20
  "Priority of the mark on the location being visited.

Above `manpage-emphasis-priority', which is 10: a compilation buffer has no
emphasis of its own, and the number says which wins if anything ever puts both
in one buffer. The same height the built-in navigation puts its own marks at,
because they are the same mark -- see the faces at the foot of this file.")

;; Which buffer `M-g n' walks. Asked from another buffer, so it cannot be read
;; off the screen the way the command can.
(setq compilation--buffer nil)

;; ---------------------------------------------------------------------------
;; Reading a line as a location
;; ---------------------------------------------------------------------------

(defun compilation--group (line index)
  "Group INDEX of the last match against LINE, or nil if INDEX is nil or the
group did not take part."
  (if (null index) nil (match-string index line)))

(defun compilation--parse (line)
  "(FILE LINE COLUMN) for LINE, or nil if it names no place.

COLUMN is nil for a format that has none. The patterns are tried in order and
the first to match decides, so the more specific ones come first in
`compilation-patterns'."
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
  "The location named on the cursor's line, or nil."
  (compilation--parse (current-line)))

;; ---------------------------------------------------------------------------
;; Going there
;; ---------------------------------------------------------------------------

(defun compilation--mark-here ()
  "Mark the cursor's line in the compilation buffer as the one being visited.

By category, so the previous mark goes when this one arrives -- which is what
makes a list of forty errors legible: exactly one line is ever marked."
  (remove-overlays 'results-here)
  (let ((start (progn (beginning-of-line) (point)))
        (end (progn (end-of-line) (point))))
    (goto-char start)
    (if (< start end)
        (make-overlay start end 'results-here compilation-mark-priority
                      'results-here))))

(defun compilation--mark-target ()
  "Mark the line point is on in the file that was jumped to."
  (remove-overlays 'results-target)
  (let ((start (progn (beginning-of-line) (point)))
        (end (progn (end-of-line) (point))))
    (goto-char start)
    (if (< start end)
        (make-overlay start end 'results-target compilation-mark-priority
                      'results-target))))

(defun compilation--visit (location)
  "Open LOCATION -- (FILE LINE COLUMN) -- in a window beside this one.

The compilation output stays visible, which is the point of having it in a
buffer at all: you read the next complaint without going back for it."
  (let ((file (nth 0 location))
        (line (string-to-number (nth 1 location)))
        (column (nth 2 location))
        (from (selected-window)))
    (compilation--mark-here)
    (setq compilation--buffer (current-buffer))
    ;; Reuse the window last jumped into, the way dired's `o' does, so walking
    ;; a list of errors does not slice the frame into eight.
    (if (and compilation--file-window (select-window compilation--file-window))
        nil
        (progn
          (split-window-below compilation-window-height)
          (setq compilation--file-window (selected-window))))
    ;; A relative path resolves against the editor's working directory, which
    ;; is right when the editor was started where the command was run -- the
    ;; normal case. When it is not, the file is simply not found and this says
    ;; so, rather than opening some other file of the same name.
    (if (file-exists-p (expand-file-name file))
        (progn
          (find-file file)
          (goto-line line)
          (beginning-of-line)
          (if column (forward-char (- (string-to-number column) 1)))
          (compilation--mark-target)
          (select-window from))
        (progn
          (select-window from)
          (message "No such file: %s" file)))))

(setq compilation--file-window nil)

(defcommand compilation-goto () nil
  "Open what the cursor's line names. Bound to RET in compilation-mode."
  (let ((location (compilation--location-here)))
    (if location
        (progn
          ;; This listing is the one being walked now, so that `M-g n' from
          ;; the file carries on from the line just opened rather than from
          ;; whatever was searched for last.
          (compilation--rebuild (current-buffer))
          (compilation--visit location))
        (message "No file and line on this line"))))

;; ---------------------------------------------------------------------------
;; Walking the complaints
;; ---------------------------------------------------------------------------

(defun compilation--step (direction)
  "Move to the next line naming a location, DIRECTION being 1 or -1.

Returns t if it found one, leaving point there; nil if it ran out, leaving
point where it started."
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
  "Move to the next line naming a location. Bound to n in compilation-mode."
  (if (not (compilation--step 1)) (message "No more")))

(defcommand compilation-previous () nil
  "Move to the previous line naming a location. Bound to p."
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
  "Every location this buffer names, in the shape a result set takes.

(KIND SOURCE LINE COLUMN) is all a compiler's complaint can say; the rest of an
entry -- the offset, the text of the line, where the match sits in it -- is for
things that searched text, and is left off."
  (let ((line 1)
        (last (progn (end-of-buffer) (line-number-at-point)))
        (entries nil))
    (while (<= line last)
      (goto-line line)
      (let ((location (compilation--parse (current-line))))
        (if location
            (setq entries
                  (cons (list "file"
                              (nth 0 location)
                              (if (nth 1 location) (string-to-number (nth 1 location)) 1)
                              (if (nth 2 location)
                                  (- (string-to-number (nth 2 location)) 1)
                                  0))
                        entries))))
      (setq line (+ line 1)))
    (reverse entries)))

(defun compilation--rebuild (buffer)
  "Attach what BUFFER complains about, as a result set. The refresher."
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
  "The command this buffer's output came from, read off its first line.

`shell-command-start' writes `$ <command>' as the first line, so the buffer
says what produced it. Reading it back is the same trick `dired' uses for the
directory it is listing: a variable beside the buffer can disagree with what is
displayed, and reading the screen cannot."
  (let ((here (point)))
    (goto-line 1)
    (let ((line (current-line)))
      (goto-char here)
      (if (and (> (length line) 2) (string= "$ " (substring line 0 2)))
          (substring line 2)
          nil))))

(defcommand compile (command) ("sCompile command: ")
  "Run COMMAND and show what it says, ready to be jumped around.

Bound to C-c c. The command runs in the background, so the editor stays usable
while it works and output appears as it is produced. Return on a line naming a
file and line opens it beside the output; `M-g n' and `M-g p' walk them from
anywhere."
  (let ((name (shell-command-start command 'compilation-mode)))
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
          (display-buffer-at-bottom name compilation-window-height)))))

(defcommand compilation-recompile () nil
  "Run this buffer's command again. Bound to g in compilation-mode."
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
(define-key 'compilation-mode "q" 'delete-window)

;; The exit line, as in `shell-output-mode': a failure should be visible at a
;; glance rather than read.
(add-syntax-rule 'compilation-mode "^--- exited 0 ---$" 'comment)
(add-syntax-rule 'compilation-mode "^--- exited [1-9][0-9]* ---$" 'error)
(add-syntax-rule 'compilation-mode "^\\$ .*$" 'keyword)

;; The two marks. Backgrounds rather than foregrounds: the text under them is
;; already coloured by the rules above, and a background says "you are here"
;; without arguing with what the text is.
(set-face 'results-here nil "bright-black")
(set-face 'results-target nil "bright-black")

(log "compile loaded")
