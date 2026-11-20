;;; eshell --- a shell in a buffer, and Lisp at its prompt.
;;;
;;; M-x eshell runs bash, sh, cmd or PowerShell in *eshell*: what it prints
;;; comes in as it is printed, Return sends what was typed after it, and a
;;; line that starts with a comma is evaluated as Lisp instead.
;;; User manual: the `eshell' page.
;;; Design: `doc/architettura-editor.typ, <mod-eshell>`.

(require 'commands)
(require 'debug)

(make-mode 'eshell-mode)

;; ---------------------------------------------------------------------------
;; Settings
;; ---------------------------------------------------------------------------

(defvar eshell-shells
  (list
   (list 'bash
         :program "bash"
         :args '("--noediting" "-i")
         ;; The prompt says where the shell is with OSC 7, which the output
         ;; filter reads and takes out: see `eshell--escapes'.
         :environment
         '("TERM=dumb" "PAGER=cat" "GIT_PAGER=cat"
           "PROMPT_COMMAND=printf '\\e]7;file://%s%s\\e\\\\' \"$HOSTNAME\" \"$PWD\""))
   (list 'sh
         :program "sh"
         ;; No line editing: bash as sh would print back what it reads.
         :args '("-i" "+o" "emacs")
         ;; Each sh reads the backslashes of a prompt its own way -- dash
         ;; takes one away, bash as sh two -- and none those of a value: the
         ;; ESC \ that ends the sequence comes from a variable.
         :environment
         '("TERM=dumb" "PAGER=cat" "GIT_PAGER=cat" "RSEDIT_OSC_END=\e\\"
           "PS1=\e]7;file://${PWD}${RSEDIT_OSC_END}$ "))
   (list 'cmd
         :program "cmd.exe"
         ;; UTF-8 in and out, before the first prompt; the banner is left out.
         :args '("/K" "chcp 65001>nul")
         :environment '("PROMPT=$E]7;file:///$P$E\\$P$G")
         :newline "\r\n"
         :echoes t
         :prompt-regexp "^[A-Za-z]:[^>]*> *")
   (list 'powershell
         :program "powershell.exe"
         :args '("-NoLogo" "-NoProfile" "-Command" "-")
         :newline "\r\n"
         :init '("[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)")
         ;; Read from a pipe, PowerShell shows no prompt: it is asked for one
         ;; after each line.
         :prompt "Write-Host -NoNewline \"PS $($PWD.ProviderPath)> \""
         :prompt-regexp "^PS [^>]*> *")
   (list 'pwsh
         :program "pwsh"
         :args '("-NoLogo" "-NoProfile" "-Command" "-")
         :init '("[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)")
         :prompt "Write-Host -NoNewline \"PS $($PWD.ProviderPath)> \""
         :prompt-regexp "^PS [^>]*> *"))
  "The shells `eshell' can run, each a list of its name and a property list:
:program and :args, how it is started; :environment, variables given it as
\"NAME=VALUE\"; :newline, what ends a line sent to it, \"\\n\" unless given;
:init, lines sent once it has started; :prompt, a line sent after each one
typed, for a shell that prints no prompt; :echoes, non-nil for a shell that
prints back each line it reads; :prompt-regexp, what a prompt looks like.")

(defvar eshell-shell nil
  "The name of the shell in `eshell-shells' that `eshell' runs: nil for cmd
on Windows, and elsewhere the one named as $SHELL is, or sh.")

(defvar eshell-buffer-name "*eshell*"
  "The name of the buffer `eshell' opens.")

(defvar eshell-lisp-prefix ","
  "What starts a line evaluated as Lisp rather than sent to the shell; nil
sends every line to the shell.")

(defvar eshell-history-size 500
  "How many lines the history of `eshell' keeps.")

(defvar eshell-history-duplicates 'consecutive
  "Which repeats the history of `eshell' keeps: all; consecutive, the
default, to keep all but a line just like the one before; or erase, to keep
only the newest copy of each line.")

(defvar eshell-history-ignore nil
  "A regexp: a line it matches is not kept in the history of `eshell'.")

(defvar eshell-history-save t
  "Non-nil to keep the history of `eshell' from one run to the next.")

(define-face 'eshell-input nil nil '("bold"))

;; ---------------------------------------------------------------------------
;; Profiles
;; ---------------------------------------------------------------------------

(defun eshell--property (profile key)
  "The value under KEY in the property list PROFILE, or nil."
  (let ((rest profile)
        (found nil))
    (while rest
      (if (eq (car rest) key)
          (progn (setq found (car (cdr rest)))
                 (setq rest nil))
          (setq rest (cdr (cdr rest)))))
    found))

(defun eshell--get (key buffer)
  "The value kept under KEY with BUFFER, or with the current buffer when nil."
  (if buffer (buffer-get key buffer) (buffer-get key)))

(defun eshell--profile (&optional buffer)
  "The properties of the shell BUFFER runs, the current buffer by default."
  (cdr (assq (eshell--get 'eshell--shell buffer) eshell-shells)))

(defun eshell--windows-p ()
  "Non-nil on Windows, where the directories of a path are split by ;."
  (string= (path-separator) ";"))

(defun eshell--default-shell ()
  "The name of the shell `eshell' runs when it is not told which."
  (cond (eshell-shell eshell-shell)
        ((eshell--windows-p) 'cmd)
        (t (let ((login (getenv "SHELL")))
             (let ((named (and login (intern (file-name-nondirectory login)))))
               (if (and named (assq named eshell-shells)) named 'sh))))))

(defun eshell--newline (profile)
  "What ends a line sent to the shell PROFILE describes."
  (or (eshell--property profile :newline) "\n"))

;; ---------------------------------------------------------------------------
;; Text
;; ---------------------------------------------------------------------------

(defun eshell--join (parts separator)
  "PARTS, a list of strings, joined with SEPARATOR between them."
  (let ((joined (car parts)))
    (dolist (part (cdr parts))
      (setq joined (concat joined separator part)))
    (or joined "")))

(defun eshell--blank-p (text)
  "Non-nil when TEXT has nothing but blanks."
  (string-match-p "^[ \\t\\n]*$" text))

(defun eshell--line-start (position)
  "The position of the start of the line POSITION is on."
  (let ((here (point)))
    (goto-char position)
    (beginning-of-line)
    (let ((start (point)))
      (goto-char here)
      start)))

(defun eshell--directory (payload)
  "The directory an OSC 7 PAYLOAD names, or nil: file://HOST/PATH or PATH."
  (let ((path (if (string-prefix-p "file://" payload)
                  (let ((rest (substring payload 7)))
                    (if (string-match "/" rest) (substring rest (match-beginning 0)) ""))
                  payload)))
    ;; file:///C:\Users, as cmd writes it, is the directory C:\Users.
    (if (string-match-p "^/[A-Za-z]:" path) (substring path 1) path)))

(defun eshell--escapes (text)
  "TEXT without its terminal escape sequences; one that says where the shell
is makes it the buffer's directory, and one cut short at the end of TEXT is
kept for the next output."
  (let ((clean "")
        (from 0)
        (done nil))
    (while (not done)
      (let ((at (string-match "\e" text from)))
        (if (null at)
            (progn (setq clean (concat clean (substring text from)))
                   (setq done t))
            (let ((tail (substring text at))
                  (skip nil))
              (setq clean (concat clean (substring text from at)))
              (cond
               ;; An operating system command, as OSC 7. It ends at BEL or at
               ;; ESC \; an ESC followed by anything else ends it too, as a
               ;; terminal ends it, and starts what follows.
               ((string-match "^\e\\]([^\\x07\\x1b]*)(\\x07|\e\\\\|\e[^\\\\])" tail)
                (let ((payload (match-string 1 tail))
                      (ender (match-string 2 tail)))
                  (setq skip (if (and (= (length ender) 2) (not (string= ender "\e\\")))
                                 (- (match-end 0) 2)
                                 (match-end 0)))
                  (when (string-prefix-p "7;" payload)
                    (let ((directory (eshell--directory (substring payload 2))))
                      (when (and directory (file-directory-p directory))
                        (set-buffer-directory directory))))))
               ;; A control sequence: colours, the cursor, erasing.
               ((string-match "^\e\\[[0-?]*[ -/]*[@-~]" tail)
                (setq skip (match-end 0)))
               ;; One of the two cut short: kept, unless too long to be one.
               ((and (string-match-p "^\e(\\[[0-?]*[ -/]*|\\][^\\x07\\x1b]*\e?)?$" tail)
                     (< (length tail) 4096))
                (buffer-put 'eshell--pending tail)
                (setq done t))
               ;; A character set chosen, as ESC ( B.
               ((string-match "^\e[ -/]+[0-~]" tail)
                (setq skip (match-end 0)))
               ;; Two characters, as ESC c or ESC =.
               ((string-match-p "^\e[0-~]" tail)
                (setq skip 2))
               ;; An escape that starts nothing: it goes, and what follows it
               ;; stays.
               (t (setq skip 1)))
              (if skip
                  (setq from (+ at skip))
                  (setq done t))))))
    clean))

(defun eshell--unecho (text)
  "TEXT without the copy of the line sent that a shell which echoes prints."
  (let ((echo (buffer-get 'eshell--echo)))
    (cond ((or (null echo) (string= echo "")) text)
          ((string-prefix-p text echo)
           (buffer-put 'eshell--echo (substring echo (length text)))
           "")
          ((string-prefix-p echo text)
           (buffer-put 'eshell--echo nil)
           (substring text (length echo)))
          (t (buffer-put 'eshell--echo nil)
             text))))

;; ---------------------------------------------------------------------------
;; Output
;; ---------------------------------------------------------------------------

(defun eshell--marker-position ()
  "Where the output goes in the current buffer, and the input starts."
  (marker-position (buffer-get 'eshell--marker)))

(defun eshell--insert (text)
  "Insert TEXT where the output goes in the current buffer: point stays on
what it was on, and with what is being typed when it is there."
  (unless (string= text "")
    (let ((from (eshell--marker-position))
          (here (point)))
      (goto-char from)
      (insert text)
      (let ((to (point)))
        (set-marker (buffer-get 'eshell--marker) to)
        (goto-char (if (>= here from) (+ here (- to from)) here))))))

(defun eshell--output (text)
  "Insert TEXT, what the shell printed, as the buffer should show it."
  (let ((joined (concat (or (buffer-get 'eshell--pending) "") text)))
    ;; A \r at the end may be the first half of a \r\n: it waits for the next.
    (let ((held (if (string-suffix-p "\r" joined) "\r" ""))
          (body (if (string-suffix-p "\r" joined)
                    (substring joined 0 (- (length joined) 1))
                    joined)))
      (buffer-put 'eshell--pending nil)
      (eshell--insert
       (eshell--escapes (eshell--unecho (eshell--join (split-string body "\r\n") "\n"))))
      (unless (string= held "")
        (buffer-put 'eshell--pending (concat (or (buffer-get 'eshell--pending) "") held))))))

(defun eshell--at-line-start-p ()
  "Non-nil when the output ends a line, or there is none."
  (let ((at (eshell--marker-position)))
    (or (= at 0) (string= (buffer-substring (- at 1) at) "\n"))))

(defun eshell--exited (status)
  "Say how the shell ended, with STATUS as `process-status' says it."
  (buffer-put 'eshell--process nil)
  (eshell--insert
   (concat (if (eshell--at-line-start-p) "" "\n")
           (if (consp status)
               (format "--- exited %s ---" (nth 1 status))
               "--- killed ---")
           "\n")))

(defun eshell--receive (name kind value)
  "Take VALUE, of KIND, from the shell NAME into its buffer."
  (when (and (member name (all-buffer-names))
             (equal (buffer-get 'eshell--process name) name))
    (with-current-buffer
     name
     (lambda ()
       (cond ((eq kind 'output) (eshell--output value))
             ((eq kind 'error) (eshell--insert (concat value "\n")))
             (t (eshell--exited value)))))))

;; ---------------------------------------------------------------------------
;; Starting a shell
;; ---------------------------------------------------------------------------

(defun eshell--free-buffer-name ()
  "A name for a new shell's buffer that no buffer has."
  (let ((name eshell-buffer-name)
        (count 1))
    (while (member name (all-buffer-names))
      (setq count (+ count 1))
      (setq name (concat eshell-buffer-name "<" (number-to-string count) ">")))
    name))

(defun eshell-process (&optional buffer)
  "Return the handle of the shell running in BUFFER, the current buffer by
default: the name `process-send', `process-signal' and `process-status' take.
Return nil when no shell runs there, or once the one there has said how it
ended."
  (eshell--get 'eshell--process buffer))

(defun eshell--launch (shell profile directory)
  "Start SHELL, as PROFILE says, in the current buffer and in DIRECTORY, or
the buffer's own: the handle, or nil."
  (unless (eq (major-mode) 'eshell-mode)
    (set-major-mode 'eshell-mode))
  (when directory
    (set-buffer-directory directory))
  (let ((marker (buffer-get 'eshell--marker)))
    (if (and marker (marker-position marker))
        (set-marker marker (point-max))
        (buffer-put 'eshell--marker (make-marker (point-max)))))
  (goto-char (point-max))
  (buffer-put 'eshell--shell shell)
  (buffer-put 'eshell--echo nil)
  (buffer-put 'eshell--pending nil)
  (buffer-put 'eshell--walk nil)
  (let ((handle (process-start (current-buffer)
                               (eshell--property profile :program)
                               (eshell--property profile :args)
                               'eshell--receive 'chunks
                               :errors 'merged
                               :environment (eshell--property profile :environment))))
    (when handle
      (buffer-put 'eshell--process handle)
      (dolist (line (eshell--property profile :init))
        (process-send handle (concat line (eshell--newline profile))))
      (let ((prompt (eshell--property profile :prompt)))
        (when prompt
          (process-send handle (concat prompt (eshell--newline profile))))))
    handle))

(defun eshell-start (&optional shell buffer)
  "Start SHELL, a name in `eshell-shells', the default shell when nil, in
BUFFER, a new buffer when nil, without showing it, and return the handle of
the shell's process: the name `process-send', `process-signal' and
`process-status' take. Return nil, saying why, when it cannot start.

Example:
  (process-send (eshell-start 'bash) \"ls\\n\")"
  (let ((shell (or shell (eshell--default-shell)))
        (buffer (or buffer (eshell--free-buffer-name)))
        (directory (working-directory)))
    (let ((profile (cdr (assq shell eshell-shells))))
      (cond
       ((null profile)
        (message "No shell is called %s in eshell-shells" shell)
        nil)
       ((eshell-process buffer)
        (message "A shell is running in %s already" buffer)
        nil)
       ((member buffer (all-buffer-names))
        ;; A shell's own buffer keeps the directory it got to.
        (with-current-buffer
         buffer
         (lambda ()
           (eshell--launch shell profile
                           (if (eq (major-mode) 'eshell-mode) nil directory)))))
       (t
        (buffer-create buffer :mode 'eshell-mode :role 'editing)
        (with-current-buffer
         buffer
         (lambda () (eshell--launch shell profile directory))))))))

(defun eshell--open (shell buffer)
  "Show BUFFER, a new buffer when nil, with SHELL running in it: started
when none is, the one it ran last or the default when SHELL is nil."
  (let ((buffer (or buffer (eshell--free-buffer-name))))
    (if (eshell-process buffer)
        (progn (switch-to-buffer buffer) (eshell-process buffer))
        (let ((handle (eshell-start
                       (or shell
                           (and (member buffer (all-buffer-names))
                                (buffer-get 'eshell--shell buffer)))
                       buffer)))
          (when handle
            (switch-to-buffer buffer))
          handle))))

(defun eshell--shell-candidates (input)
  "The names of the shells in `eshell-shells' that INPUT matches."
  (fuzzy-filter input (mapcar (lambda (entry) (symbol-name (car entry))) eshell-shells)))

(defcommand eshell (&optional arg) ("P")
  "Switch to *eshell*, and start a shell in it when none is running there:
the shell it ran last, or `eshell-shell'. With a prefix argument, ask which
shell, and start it in a buffer of its own. Return the shell's handle."
  (if arg
      (minibuffer-read "Shell:"
                       (lambda (name) (eshell--open (intern name) nil))
                       'eshell--shell-candidates)
      (eshell--open nil eshell-buffer-name)))

;; ---------------------------------------------------------------------------
;; History
;; ---------------------------------------------------------------------------

;; Newest first, shared by every shell's buffer. Read from the session the
;; first time it is needed: the session is read after init.lisp.
(setq eshell--history nil)
(setq eshell--history-read nil)

(defun eshell--load-history ()
  "Read back the history the last run kept, the first time it is asked for."
  (unless eshell--history-read
    (setq eshell--history-read t)
    (when eshell-history-save
      (let ((kept (session-value 'eshell-history nil)))
        (when (listp kept)
          (setq eshell--history (append eshell--history kept)))))))

(defun eshell--save-history ()
  "Keep the history for the next run, when `eshell-history-save' says to."
  (when eshell-history-save
    (set-session-value 'eshell-history eshell--history)))

(defun eshell--first (count items)
  "The first COUNT of ITEMS."
  (let ((kept nil))
    (while (and items (> count 0))
      (setq kept (cons (car items) kept))
      (setq items (cdr items))
      (setq count (- count 1)))
    (reverse kept)))

(defun eshell--without (item items)
  "ITEMS without any copy of ITEM."
  (let ((kept nil))
    (dolist (each items)
      (unless (equal each item)
        (setq kept (cons each kept))))
    (reverse kept)))

(defun eshell--remember (line)
  "Add LINE to the history, as the history settings say."
  (eshell--load-history)
  (unless (or (eshell--blank-p line)
              (and eshell-history-ignore (string-match-p eshell-history-ignore line))
              (and (memq eshell-history-duplicates '(consecutive erase))
                   (equal line (car eshell--history))))
    (when (eq eshell-history-duplicates 'erase)
      (setq eshell--history (eshell--without line eshell--history)))
    (setq eshell--history
          (eshell--first eshell-history-size (cons line eshell--history)))
    (eshell--save-history)))

(defun eshell-history ()
  "Return the lines typed at the shells of `eshell', newest first."
  (eshell--load-history)
  eshell--history)

(defcommand eshell-history-delete-duplicates () nil
  "Delete from the history of `eshell' every line typed again later, keeping
the newest copy of each, and say how many went."
  (eshell--load-history)
  (let ((kept nil))
    (dolist (line eshell--history)
      (unless (member line kept)
        (setq kept (cons line kept))))
    (let ((removed (- (length eshell--history) (length kept))))
      (setq eshell--history (reverse kept))
      (eshell--save-history)
      (message "%s duplicates deleted from the history" removed)
      removed)))

(defcommand eshell-history-clear () nil
  "Forget every line of the history of `eshell', in this run and the next."
  (eshell--load-history)
  (setq eshell--history nil)
  (eshell--save-history)
  (message "The shell history is empty"))

;; ---------------------------------------------------------------------------
;; The input
;; ---------------------------------------------------------------------------

(defun eshell--input ()
  "What has been typed after the prompt."
  (buffer-substring (eshell--marker-position) (point-max)))

(defun eshell--replace-input (text)
  "Put TEXT after the prompt, in place of what was typed there."
  (delete-region (eshell--marker-position) (point-max))
  (goto-char (point-max))
  (insert text))

(defun eshell--live-shell ()
  "The handle of the shell of this buffer, or nil, saying why."
  (let ((handle (eshell-process)))
    (cond (handle handle)
          ((null (buffer-get 'eshell--marker))
           (message "This is not a shell's buffer")
           nil)
          (t (message "No shell is running here: M-x eshell starts one")
             nil))))

(defun eshell--lisp-p (line)
  "Non-nil when LINE is to be evaluated as Lisp."
  (and eshell-lisp-prefix
       (not (string= eshell-lisp-prefix ""))
       (string-prefix-p eshell-lisp-prefix line)))

(defun eshell--print (value)
  "VALUE as the result of a Lisp line shows it: a string as it is."
  (if (stringp value) value (prin1-to-string value)))

(defun eshell--evaluate (source prompt)
  "Evaluate SOURCE, and write its value, then PROMPT again for the next line."
  (let ((buffer (current-buffer))
        (outcome (if (eshell--blank-p source)
                     (list nil "nothing to evaluate")
                     (eval-string-top-level source))))
    (let ((text (if (car outcome)
                    (eshell--print (nth 1 outcome))
                    (format "error: %s" (nth 1 outcome)))))
      ;; What was evaluated may have made another buffer current, or closed
      ;; this one.
      (when (member buffer (all-buffer-names))
        (with-current-buffer
         buffer
         (lambda ()
           (eshell--insert (concat text (if (string-suffix-p "\n" text) "" "\n") prompt))))))))

(defun eshell--send (handle line)
  "Send LINE to the shell HANDLE, as its profile says lines are sent."
  (let ((profile (eshell--profile)))
    (let ((newline (eshell--newline profile))
          (prompt (eshell--property profile :prompt)))
      (when (eshell--property profile :echoes)
        (buffer-put 'eshell--echo (concat line "\n")))
      (if (process-send handle (concat (eshell--join (split-string line "\n") newline) newline))
          (when prompt
            (process-send handle (concat prompt newline)))
          (message "The shell is not reading: the line did not go")))))

(defun eshell--copy-old-input ()
  "Put the line point is on, without its prompt, after the last prompt."
  (let ((line (current-line))
        (regexp (or (eshell--property (eshell--profile) :prompt-regexp)
                    "^[^#$%>\\n]*[#$%>] *")))
    (eshell--replace-input
     (if (string-match regexp line) (substring line (match-end 0)) line))))

(defcommand eshell-send-input () nil
  "Send the line typed after the prompt to the shell, or evaluate it when it
starts with `eshell-lisp-prefix'; on an earlier line, copy that line, without
its prompt, after the last prompt."
  (cond
   ((null (buffer-get 'eshell--marker))
    (message "This is not a shell's buffer"))
   ((< (point) (eshell--marker-position))
    (eshell--copy-old-input))
   (t
    (let ((line (eshell--input))
          (start (eshell--marker-position)))
      (let ((lisp (eshell--lisp-p line))
            (handle (eshell-process)))
        (when (or lisp (eshell--live-shell))
          (let ((prompt (buffer-substring (eshell--line-start start) start)))
            (goto-char (point-max))
            (insert "\n")
            (when (> (length line) 0)
              (make-overlay start (+ start (length line)) 'eshell-input 0 'eshell-input))
            (set-marker (buffer-get 'eshell--marker) (point-max))
            (buffer-put 'eshell--walk nil)
            (eshell--remember line)
            (if lisp
                (eshell--evaluate (substring line (length eshell-lisp-prefix)) prompt)
                (eshell--send handle line)))))))))

(defun eshell--recall (step)
  "Put after the prompt the line STEP away in the history from the one shown."
  (eshell--load-history)
  (when (buffer-get 'eshell--marker)
    (let ((walk (buffer-get 'eshell--walk)))
      (let ((index (+ (if walk walk -1) step)))
        (cond
         ((>= index (length eshell--history))
          (message "No earlier line in the history"))
         ((< index 0)
          (if walk
              (progn (eshell--replace-input (or (buffer-get 'eshell--stash) ""))
                     (buffer-put 'eshell--walk nil))
              (message "No later line in the history")))
         (t
          (unless walk
            (buffer-put 'eshell--stash (eshell--input)))
          (eshell--replace-input (nth index eshell--history))
          (buffer-put 'eshell--walk index)))))))

(defcommand eshell-previous-input () nil
  "Put the line typed before the one after the prompt in its place."
  (eshell--recall 1))

(defcommand eshell-next-input () nil
  "Put the line typed after the one after the prompt in its place, and at the
end what was being typed."
  (eshell--recall -1))

(defun eshell--history-candidates (input)
  "The lines of the history that INPUT matches, best first."
  (fuzzy-filter input (eshell-history)))

(defcommand eshell-search-history () nil
  "Ask for a line of the history, completing over it, and put it after the
prompt."
  (let ((buffer (current-buffer)))
    (if (buffer-get 'eshell--marker)
        (minibuffer-read "Shell history:"
                         (lambda (line)
                           (when (member buffer (all-buffer-names))
                             (with-current-buffer
                              buffer
                              (lambda () (eshell--replace-input line)))))
                         'eshell--history-candidates)
        (message "This is not a shell's buffer"))))

(defcommand eshell-kill-input () nil
  "Delete what has been typed after the prompt."
  (when (buffer-get 'eshell--marker)
    (eshell--replace-input "")))

(defcommand eshell-beginning-of-line () nil
  "Move to the start of the line, or, on the prompt's line, to where the
input starts."
  (let ((marker (buffer-get 'eshell--marker)))
    (if (and marker
             (>= (point) (marker-position marker))
             (= (eshell--line-start (point)) (eshell--line-start (marker-position marker))))
        (goto-char (marker-position marker))
        (beginning-of-line))))

(defcommand eshell-clear () nil
  "Delete everything above the prompt's line."
  (when (buffer-get 'eshell--marker)
    (delete-region 0 (eshell--line-start (eshell--marker-position)))))

;; ---------------------------------------------------------------------------
;; Signals, the end of the input, stopping
;; ---------------------------------------------------------------------------

(defcommand eshell-interrupt () nil
  "Interrupt what the shell is running, as C-c does at a terminal: the shell
stays. Where the system cannot, as on Windows, say so."
  (let ((handle (eshell--live-shell)))
    (when handle
      (buffer-put 'eshell--echo nil)
      (unless (process-signal handle 'interrupt)
        (message "A shell cannot be interrupted here: C-c C-k stops it")))))

(defcommand eshell-send-eof () nil
  "Close the input of the shell, as C-d does at a terminal: most shells end."
  (let ((handle (eshell--live-shell)))
    (when handle
      (process-close-input handle))))

(defcommand eshell-kill () nil
  "Stop the shell of this buffer, and everything it started. The buffer
stays, and ends with --- killed ---."
  (let ((handle (eshell--live-shell)))
    (when handle
      (process-stop handle))))

(defun eshell--closed ()
  "Stop the shell of the buffer just closed."
  (process-stop closed-buffer-name))

(add-hook 'eshell-mode "after-close-hook" 'eshell--closed)

;; ---------------------------------------------------------------------------
;; Keys, menus, colours
;; ---------------------------------------------------------------------------

(define-key 'eshell-mode "<ret>" 'eshell-send-input)
(define-key 'eshell-mode "M-p" 'eshell-previous-input)
(define-key 'eshell-mode "M-n" 'eshell-next-input)
(define-key 'eshell-mode "M-r" 'eshell-search-history)
(define-key 'eshell-mode "C-a" 'eshell-beginning-of-line)
(define-key 'eshell-mode "C-c C-c" 'eshell-interrupt)
(define-key 'eshell-mode "C-c C-d" 'eshell-send-eof)
(define-key 'eshell-mode "C-c C-k" 'eshell-kill)
(define-key 'eshell-mode "C-c C-u" 'eshell-kill-input)
(define-key 'eshell-mode "C-c M-o" 'eshell-clear)

(menu-bar-add '("Eshell" "Interrupt") 'eshell-interrupt nil :mode 'eshell-mode)
(menu-bar-add '("Eshell" "Send End of Input") 'eshell-send-eof nil :mode 'eshell-mode)
(menu-bar-add '("Eshell" "Stop the Shell") 'eshell-kill nil :mode 'eshell-mode)
(menu-bar-add '("Eshell" "--history") nil nil :mode 'eshell-mode)
(menu-bar-add '("Eshell" "Search History...") 'eshell-search-history nil :mode 'eshell-mode)
(menu-bar-add '("Eshell" "Delete Duplicates in History") 'eshell-history-delete-duplicates nil
              :mode 'eshell-mode)
(menu-bar-add '("Eshell" "--buffer") nil nil :mode 'eshell-mode)
(menu-bar-add '("Eshell" "Clear") 'eshell-clear nil :mode 'eshell-mode)

(add-syntax-rule 'eshell-mode "^--- exited 0 ---$" 'comment)
(add-syntax-rule 'eshell-mode "^--- exited [1-9][0-9]* ---$" 'error)
(add-syntax-rule 'eshell-mode "^--- killed ---$" 'error)

(log "eshell loaded")
(provide 'eshell)
