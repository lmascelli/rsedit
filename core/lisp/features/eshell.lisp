;;; eshell --- a shell in a buffer, and Lisp at its prompt.
;;;
;;; M-x eshell runs bash, sh, cmd or PowerShell in *eshell*: what it prints
;;; comes in as it is printed, Return sends what was typed after it, and a
;;; line that starts with a comma is evaluated as Lisp instead.
;;; User manual: the `eshell' page.
;;; Design: `doc/architettura-editor.typ, <mod-eshell>`.

(require 'commands)
(require 'debug)
(require 'completion-at-point)

(make-mode 'eshell-mode)

;; ---------------------------------------------------------------------------
;; Settings
;; ---------------------------------------------------------------------------

;; The commands of PowerShell that are not programs: the cmdlets used most, and
;; the aliases it gives them.
(defconst eshell--powershell-commands
  '("Add-Content" "Copy-Item" "ForEach-Object" "Get-ChildItem" "Get-Command" "Get-Content"
    "Get-Date" "Get-Help" "Get-Item" "Get-Location" "Get-Process" "Get-Service"
    "Invoke-WebRequest" "Measure-Object" "Move-Item" "New-Item" "Out-File" "Remove-Item"
    "Rename-Item" "Select-Object" "Select-String" "Set-Content" "Set-Location" "Sort-Object"
    "Stop-Process" "Test-Path" "Where-Object" "Write-Host" "Write-Output"
    "cat" "cd" "clear" "cls" "copy" "del" "dir" "echo" "ls" "mkdir" "move" "pwd" "rm" "type")
  "The commands of PowerShell that are not on the PATH.")

(defvar eshell-shells
  (list
   (list 'bash
         :program "bash"
         :args '("--noediting" "-i")
         ;; The prompt says where the shell is with OSC 7, which the output
         ;; filter reads and takes out: see `eshell--escapes'.
         :environment
         '("TERM=dumb" "PAGER=cat" "GIT_PAGER=cat"
           "PROMPT_COMMAND=printf '\\e]7;file://%s%s\\e\\\\' \"$HOSTNAME\" \"$PWD\"")
         :commands
         '("alias" "bg" "bind" "builtin" "cd" "command" "declare" "dirs" "disown" "echo"
           "enable" "eval" "exec" "exit" "export" "fc" "fg" "getopts" "hash" "help" "history"
           "jobs" "kill" "let" "local" "logout" "popd" "printf" "pushd" "pwd" "read"
           "readonly" "return" "set" "shift" "shopt" "source" "test" "times" "trap" "type"
           "typeset" "ulimit" "umask" "unalias" "unset" "wait"))
   (list 'sh
         :program "sh"
         ;; No line editing: bash as sh would print back what it reads.
         :args '("-i" "+o" "emacs")
         ;; Each sh reads the backslashes of a prompt its own way -- dash
         ;; takes one away, bash as sh two -- and none those of a value: the
         ;; ESC \ that ends the sequence comes from a variable.
         :environment
         '("TERM=dumb" "PAGER=cat" "GIT_PAGER=cat" "RSEDIT_OSC_END=\e\\"
           "PS1=\e]7;file://${PWD}${RSEDIT_OSC_END}$ ")
         :commands
         '("alias" "bg" "cd" "command" "echo" "eval" "exec" "exit" "export" "fg" "getopts"
           "hash" "jobs" "kill" "printf" "pwd" "read" "readonly" "return" "set" "shift" "test"
           "times" "trap" "type" "ulimit" "umask" "unalias" "unset" "wait"))
   (list 'cmd
         :program "cmd.exe"
         ;; UTF-8 in and out, before the first prompt; the banner is left out.
         :args '("/K" "chcp 65001>nul")
         :environment '("PROMPT=$E]7;file:///$P$E\\$P$G")
         :newline "\r\n"
         :echoes t
         :prompt-regexp "^[A-Za-z]:[^>]*> *"
         :syntax 'cmd
         :commands
         '("assoc" "call" "cd" "chdir" "cls" "color" "copy" "date" "del" "dir" "echo"
           "endlocal" "erase" "exit" "for" "ftype" "goto" "if" "md" "mkdir" "mklink" "move"
           "path" "pause" "popd" "prompt" "pushd" "rd" "rem" "ren" "rename" "rmdir" "set"
           "setlocal" "shift" "start" "time" "title" "type" "ver" "verify" "vol"))
   (list 'powershell
         :program "powershell.exe"
         :args '("-NoLogo" "-NoProfile" "-Command" "-")
         :newline "\r\n"
         :init '("[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)")
         ;; Read from a pipe, PowerShell shows no prompt: it is asked for one
         ;; after each line.
         :prompt "Write-Host -NoNewline \"PS $($PWD.ProviderPath)> \""
         :prompt-regexp "^PS [^>]*> *"
         :syntax 'powershell
         :commands eshell--powershell-commands)
   (list 'pwsh
         :program "pwsh"
         :args '("-NoLogo" "-NoProfile" "-Command" "-")
         :init '("[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)")
         :prompt "Write-Host -NoNewline \"PS $($PWD.ProviderPath)> \""
         :prompt-regexp "^PS [^>]*> *"
         :syntax 'powershell
         :commands eshell--powershell-commands))
  "The shells `eshell' can run, each a list of its name and a property list:
:program and :args, how it is started; :environment, variables given it as
\"NAME=VALUE\"; :newline, what ends a line sent to it, \"\\n\" unless given;
:init, lines sent once it has started; :prompt, a line sent after each one
typed, for a shell that prints no prompt; :echoes, non-nil for a shell that
prints back each line it reads; :prompt-regexp, what a prompt looks like;
:syntax, how it quotes, posix, cmd or powershell; :commands, the commands it
has that are not on the PATH.")

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

(defvar eshell-watch-for-password-prompt t
  "Non-nil to ask for a password in the minibuffer, without showing it, when
a program in the shell ends its output with a line that
`eshell-password-prompt-regexp' matches: what is typed goes to the program,
and is written neither in the buffer nor in the history.")

(defvar eshell-password-prompt-regexp
  "(?i)(?:^|[^[:alnum:]])(?:password|passphrase|pass phrase|passcode|passwort|contraseña|mot de passe|senha).*[:：]\\s*$"
  "A regexp matching the line a program asks for a password with: it ends
with a colon, after a word for password.")

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
        (buffer-put 'eshell--pending (concat (or (buffer-get 'eshell--pending) "") held)))))
  (eshell--password-prompt))

(defun eshell--password-prompt ()
  "The line the output ends with when it asks for a password, as
`eshell-password-prompt-regexp' says, and has not been asked for yet; nil
otherwise."
  (when (and eshell-watch-for-password-prompt
             (stringp eshell-password-prompt-regexp)
             (buffer-get 'eshell--process))
    (let ((end (eshell--marker-position)))
      (let ((start (eshell--line-start end)))
        (when (and (> end start)
                   (not (equal (buffer-get 'eshell--password-at) end)))
          (let ((line (buffer-substring start end)))
            (when (string-match-p eshell-password-prompt-regexp line)
              ;; Asked once: more output on another line, as a second try
              ;; after a wrong password, is asked for again.
              (buffer-put 'eshell--password-at end)
              line)))))))

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
  "Take VALUE, of KIND, from the shell NAME into its buffer, and ask for the
password it ends with, if it asks for one."
  (when (and (member name (all-buffer-names))
             (equal (buffer-get 'eshell--process name) name))
    (let ((asking (with-current-buffer
                   name
                   (lambda ()
                     (cond ((eq kind 'output) (eshell--output value))
                           ((eq kind 'error) (progn (eshell--insert (concat value "\n")) nil))
                           (t (progn (eshell--exited value) nil)))))))
      ;; Asked from where the user is, not from the shell's buffer: the
      ;; prompt gives the keyboard back there once it is answered.
      (when asking
        (eshell--ask-password name asking)))))

;; ---------------------------------------------------------------------------
;; Passwords
;; ---------------------------------------------------------------------------

(defun eshell--send-invisible (buffer prompt)
  "Ask, titled PROMPT, for what the shell of BUFFER is to read without its
being seen, and send it as a line; then start a line of output, as a
terminal would on Return."
  (process-send-password
   (buffer-get 'eshell--process buffer) prompt
   :newline (eshell--newline (eshell--profile buffer))
   :on-sent (lambda (sent)
              (when (and sent (member buffer (all-buffer-names)))
                (with-current-buffer
                 buffer
                 (lambda ()
                   (unless (eshell--at-line-start-p)
                     (eshell--insert "\n"))))))
   :on-cancel (lambda ()
                (message "Nothing was sent: M-x eshell-send-invisible sends it, C-c C-c interrupts"))))

(defun eshell--ask-password (name prompt)
  "Ask for the password the program in the shell NAME asks for with PROMPT,
and send it; when a prompt is open already, say how to send it later."
  (if (member "*Minibuffer*" (all-buffer-names))
      (message "%s asks for a password: M-x eshell-send-invisible there sends it" name)
      (eshell--send-invisible name
                              (if (equal (current-buffer) name)
                                  prompt
                                  (concat name ": " prompt)))))

(defcommand eshell-send-invisible () nil
  "Ask for a password, or anything else not to be seen, in the minibuffer
without showing it, and send it to the shell of this buffer as a line: it
is written neither in the buffer nor in the history. The question is the
line the output ends with, when there is one."
  (let ((handle (eshell--live-shell)))
    (when handle
      (let ((end (eshell--marker-position)))
        (let ((line (buffer-substring (eshell--line-start end) end)))
          (eshell--send-invisible (current-buffer)
                                  (if (eshell--blank-p line) "Password: " line)))))))

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
;; Completion
;; ---------------------------------------------------------------------------
;;
;; One source, `eshell-completion', and the only one of eshell-mode: it reads
;; the line as the shell will, and asks the part that knows -- the variables,
;; the commands on the PATH, a completer of the command, the files -- for what
;; can go in the word at point. Offering, matching and showing them is
;; `completion-at-point''s, as everywhere else.

;; How each kind of shell writes a line: what escapes a character, what
;; quotes, what starts another command, and how a variable is named, as
;; (OPEN . CLOSE) pairs, longest first.
(defconst eshell--syntaxes
  '((posix "\\" "'\"" ";|&()`" (("${" . "}") ("$" . "")))
    (cmd "^" "\"" "&|()" (("%" . "%")))
    (powershell "`" "'\"" ";|&(){}" (("$env:" . "") ("$" . ""))))
  "Each syntax a shell can have: (NAME ESCAPE QUOTES SEPARATORS VARIABLES).")

(defconst eshell--word-breaks " \t\n<>"
  "What ends a word outside quotes and starts none: blanks and redirections.")

(defvar eshell-completion-wrappers
  '("sudo" "doas" "env" "time" "nohup" "nice" "exec" "command" "xargs")
  "Commands that run the command after them: what follows one is completed
as a command, and its arguments as that command's.")

(defvar eshell-completers
  '(("cd" . eshell-complete-directories)
    ("pushd" . eshell-complete-directories)
    ("rmdir" . eshell-complete-directories)
    ("make" . eshell--complete-make)
    ("ssh" . eshell--complete-ssh)
    ("scp" . eshell--complete-scp)
    ("sftp" . eshell--complete-ssh)
    ("git" . eshell--complete-git))
  "What completes the arguments of a command, as (COMMAND . FUNCTION).
FUNCTION is called with the words of the command, unquoted, the command first
and the word being completed last, and returns what it can be: a list of
strings or (VALUE . DESCRIPTION) pairs, which `eshell-completion' quotes as
the shell wants them.")

(defun eshell--syntax ()
  "The syntax of the shell of this buffer, from its :syntax, posix unless given."
  (or (assq (or (eshell--property (eshell--profile) :syntax) 'posix) eshell--syntaxes)
      (assq 'posix eshell--syntaxes)))

(defun eshell--one-of (character characters)
  "Non-nil when CHARACTER, a string of one, is one of CHARACTERS."
  (let ((found nil)
        (at 0))
    (while (and (not found) (< at (length characters)))
      (when (string= character (substring characters at (+ at 1)))
        (setq found t))
      (setq at (+ at 1)))
    found))

(defun eshell--words (line syntax)
  "The words of the last command of LINE, read as SYNTAX reads them, each as
(START TEXT OPEN): where it starts in LINE, its text without quotes and
escapes, and the quote still open at the end of LINE when it started with it.
The last is the word LINE ends in, empty when it ends between two."
  (let ((escape (nth 1 syntax))
        (quotes (nth 2 syntax))
        (separators (nth 3 syntax))
        (words nil)
        (start nil)
        (text "")
        (lead nil)
        (quote nil)
        (escaped nil)
        (at 0))
    (while (< at (length line))
      (let ((c (substring line at (+ at 1))))
        (cond
         (escaped
          (setq text (concat text c))
          (setq escaped nil))
         (quote
          (if (string= c quote) (setq quote nil) (setq text (concat text c))))
         ((string= c escape)
          (unless start (setq start at))
          (setq escaped t))
         ((eshell--one-of c quotes)
          (unless start
            (setq start at)
            (setq lead c))
          (setq quote c))
         ((or (eshell--one-of c eshell--word-breaks) (eshell--one-of c separators))
          (when start
            (setq words (cons (list start text nil) words)))
          (setq start nil)
          (setq text "")
          (setq lead nil)
          (when (eshell--one-of c separators)
            (setq words nil)))
         (t
          (unless start (setq start at))
          (setq text (concat text c)))))
      (setq at (+ at 1)))
    (reverse (cons (list (or start (length line)) text (if (and lead quote) lead nil))
                   words))))

(defconst eshell--special "[\\] \t\n'\"\\\\$`&|;()<>*?\\[{}!]"
  "A regexp for a character a posix shell reads as more than itself.")

(defun eshell--escape (text syntax)
  "TEXT as SYNTAX writes it outside quotes: a posix shell's special
characters each after a backslash, the others' as they are."
  ;; Most names have none: one match decides, rather than a walk of every
  ;; name, which with all the programs of the PATH is thousands.
  (if (or (not (eq (car syntax) 'posix)) (not (string-match-p eshell--special text)))
      text
      (let ((escaped "")
            (at 0))
        (while (< at (length text))
          (let ((c (substring text at (+ at 1))))
            (setq escaped (concat escaped
                                  (if (eshell--one-of c " \t\n'\"\\$`&|;()<>*?[]{}!") "\\" "")
                                  c)))
          (setq at (+ at 1)))
        escaped)))

(defun eshell--quoted (candidates open syntax)
  "CANDIDATES, strings or (VALUE . DESCRIPTION) pairs, with each value written
as it is typed: after OPEN, the quote the word started with, or escaped."
  (mapcar (lambda (candidate)
            (let ((write (lambda (value)
                           (if open (concat open value) (eshell--escape value syntax)))))
              (if (consp candidate)
                  (cons (funcall write (car candidate)) (cdr candidate))
                  (funcall write candidate))))
          candidates))

(defun eshell--described (values description)
  "VALUES, each as (VALUE . DESCRIPTION)."
  (mapcar (lambda (value) (cons value description)) values))

(defun eshell-complete-files (word &optional directories)
  "Return the files WORD can be completed to, in its directory or the one of
the shell, each described as a file or a directory; only the directories when
DIRECTORIES is not nil. A completer of `eshell-completers' builds on it.

Example:
  (eshell-complete-files \"src/m\")"
  (let ((found nil))
    (dolist (path (capf-file-candidates word t))
      (if (string-suffix-p "/" path)
          (setq found (cons (cons path "directory") found))
          (unless directories
            (setq found (cons (cons path "file") found)))))
    (reverse found)))

(defun eshell-complete-directories (words)
  "Return the directories the last of WORDS can be completed to: the
completer of cd."
  (eshell-complete-files (car (reverse words)) t))

(defun eshell--path-commands ()
  "The names of the programs in the directories of the PATH: on Windows those
whose extension PATHEXT names."
  (let ((names nil)
        (extensions (and (eshell--windows-p)
                         (split-string (downcase (or (getenv "PATHEXT") ".com;.exe;.bat;.cmd"))
                                       ";"))))
    (dolist (directory (split-string (or (getenv "PATH") "") (path-separator)))
      (unless (string= directory "")
        (dolist (entry (list-dir directory))
          (unless (string-suffix-p "/" entry)
            (when (or (null extensions)
                      (let ((kept nil))
                        (dolist (extension extensions)
                          (when (and (not (string= extension ""))
                                     (string-suffix-p extension (downcase entry)))
                            (setq kept t)))
                        kept))
              (setq names (cons entry names)))))))
    names))

(defun eshell--commands (word)
  "What WORD, in the place of a command, can be completed to."
  (if (or (string-match-p "[/\\\\]" word) (string-prefix-p "." word) (string-prefix-p "~" word))
      (eshell-complete-files word)
      (append (eshell--described (eshell--property (eshell--profile) :commands) "builtin")
              (eshell--described (eshell--path-commands) "command"))))

(defun eshell--variables (raw syntax)
  "The variables RAW, a word as typed, can be completed to, written as SYNTAX
names them, or nil when RAW does not start like a variable."
  (let ((form nil))
    (dolist (pair (nth 4 syntax))
      (when (and (null form) (string-prefix-p (car pair) raw))
        (setq form pair)))
    (when form
      (mapcar (lambda (name) (cons (concat (car form) name (cdr form)) "variable"))
              (getenv-names)))))

(defun eshell--unwrapped (words)
  "WORDS without the commands of `eshell-completion-wrappers' in front, and
the options and variables given to them: the command they run, and its
arguments."
  (let ((rest words))
    (while (and (cdr rest) (member (nth 1 (car rest)) eshell-completion-wrappers))
      (setq rest (cdr rest))
      (while (and (cdr rest)
                  (let ((text (nth 1 (car rest))))
                    (or (string-prefix-p "-" text) (string-match-p "=" text))))
        (setq rest (cdr rest))))
    rest))

(defun eshell--complete-shell (start)
  "(START END CANDIDATES) for the word at point in the shell line that starts
at START, or nil."
  (let ((syntax (eshell--syntax))
        (line (buffer-substring start (point))))
    (let ((words (eshell--unwrapped (eshell--words line syntax))))
      (let ((current (car (reverse words)))
            (index (- (length words) 1)))
        (let ((from (+ start (nth 0 current)))
              (word (nth 1 current))
              (open (nth 2 current)))
          (let ((variables (and (not open)
                                (eshell--variables (buffer-substring from (point)) syntax))))
            (let ((candidates
                   (cond
                    (variables variables)
                    ((= index 0) (eshell--quoted (eshell--commands word) open syntax))
                    (t
                     (let ((completer (cdr (assoc (nth 1 (car words)) eshell-completers))))
                       (eshell--quoted
                        (if completer
                            (funcall completer (mapcar (lambda (each) (nth 1 each)) words))
                            (eshell-complete-files word))
                        open syntax))))))
              (when candidates
                (list from (eshell--word-end) candidates)))))))))

(defun eshell--word-end ()
  "Where the word at point ends: point, or the next blank after it."
  (let ((rest (buffer-substring (point) (point-max))))
    (if (string-match "[ \t\n]" rest)
        (+ (point) (match-beginning 0))
        (point-max))))

(defun eshell-completion ()
  "Return what can go in the word at point of a shell's line, as a completion
source returns it: the only source of eshell-mode. A Lisp line is completed
as Lisp, with file names and the editor's names; a shell line by what its
word is: a variable, a command, an argument of a command of
`eshell-completers', or a file."
  (let ((marker (buffer-get 'eshell--marker)))
    (when (and marker (>= (point) (marker-position marker)))
      (let ((start (marker-position marker)))
        (if (eshell--lisp-p (buffer-substring start (point)))
            (or (capf-file-name) (capf-symbols))
            (eshell--complete-shell start))))))

;; What a few commands take

(defun eshell--complete-make (words)
  "The targets of the makefile of the shell's directory."
  (let ((text (or (read-file-to-string "GNUmakefile")
                  (read-file-to-string "makefile")
                  (read-file-to-string "Makefile")
                  ""))
        (targets nil))
    (dolist (line (split-string text "\n"))
      (when (string-match "^([A-Za-z0-9_][^:=#%$ \t]*)[ \t]*:([^=]|$)" line)
        (let ((target (match-string 1 line)))
          (unless (member target targets)
            (setq targets (cons target targets))))))
    (eshell--described (reverse targets) "target")))

(defun eshell--hosts-of (config known)
  "The hosts CONFIG, the text of an ssh configuration, names, then those KNOWN,
the text of a known_hosts file, knows, each once; patterns and hashed names
left out."
  (let ((hosts nil))
    (dolist (line (split-string config "\n"))
      (when (string-match "^[ \t]*[Hh]ost[ \t]+(.+)$" line)
        (dolist (name (split-string (match-string 1 line) " "))
          (unless (or (string= name "") (string-match-p "[*?!]" name) (member name hosts))
            (setq hosts (cons name hosts))))))
    (dolist (line (split-string known "\n"))
      (unless (or (string= line "") (string-match-p "^[|@#]" line))
        (dolist (name (split-string (car (split-string line " ")) ","))
          (let ((host (if (string-match "^\\[([^]]+)\\]" name) (match-string 1 name) name)))
            (unless (or (string= host "") (string-match-p "^[|]" host) (member host hosts))
              (setq hosts (cons host hosts)))))))
    (reverse hosts)))

(defun eshell--ssh-hosts ()
  "The hosts ~/.ssh/config names and ~/.ssh/known_hosts knows."
  (eshell--hosts-of (or (read-file-to-string "~/.ssh/config") "")
                    (or (read-file-to-string "~/.ssh/known_hosts") "")))

(defun eshell--complete-ssh (words)
  "The hosts ssh knows of."
  (eshell--described (eshell--ssh-hosts) "host"))

(defun eshell--complete-scp (words)
  "The hosts ssh knows of, each before its :, and the local files."
  (let ((word (car (reverse words))))
    (if (string-match-p ":" word)
        nil
        (append (eshell--described (mapcar (lambda (host) (concat host ":")) (eshell--ssh-hosts))
                                   "host")
                (eshell-complete-files word)))))

(defconst eshell--git-commands
  '("add" "bisect" "blame" "branch" "checkout" "cherry-pick" "clean" "clone" "commit"
    "config" "diff" "fetch" "grep" "init" "log" "merge" "mv" "pull" "push" "rebase"
    "reflog" "remote" "reset" "restore" "revert" "rm" "show" "stash" "status" "switch"
    "tag" "worktree")
  "The commands of git offered after it.")

(defun eshell--git-refs (directory prefix)
  "The names of the refs in DIRECTORY, each after PREFIX, those in its
subdirectories with their path."
  (let ((names nil))
    (dolist (entry (list-dir directory))
      (if (string-suffix-p "/" entry)
          (setq names (append names (eshell--git-refs (concat directory entry)
                                                      (concat prefix entry))))
          (setq names (append names (list (concat prefix entry))))))
    names))

(defun eshell--git-branches ()
  "The branches and tags of the repository the shell is in."
  (let ((top (locate-dominating-file nil ".git")))
    (when (and top (file-directory-p (concat top ".git")))
      (let ((git (concat top ".git/"))
            (found nil))
        (dolist (name (eshell--git-refs (concat git "refs/heads/") ""))
          (setq found (cons (cons name "branch") found)))
        (dolist (name (eshell--git-refs (concat git "refs/tags/") ""))
          (setq found (cons (cons name "tag") found)))
        (dolist (line (split-string (or (read-file-to-string (concat git "packed-refs")) "")
                                    "\n"))
          (when (string-match "^[0-9a-f]+ refs/(heads|tags)/(.+)$" line)
            (let ((name (match-string 2 line))
                  (kind (if (string= (match-string 1 line) "heads") "branch" "tag")))
              (unless (assoc name found)
                (setq found (cons (cons name kind) found))))))
        (reverse found)))))

(defun eshell--complete-git (words)
  "The commands of git, then branches and tags where they are wanted, and
files."
  (let ((word (car (reverse words))))
    (cond ((= (length words) 2)
           (eshell--described eshell--git-commands "subcommand"))
          ((member (nth 1 words) '("checkout" "switch" "merge" "rebase" "branch" "reset"
                                   "cherry-pick" "log" "diff" "show"))
           (append (eshell--git-branches) (eshell-complete-files word)))
          (t (eshell-complete-files word)))))

(set-completion-functions 'eshell-mode '(eshell-completion) :global nil)

;; ---------------------------------------------------------------------------
;; Keys, menus, colours
;; ---------------------------------------------------------------------------

(define-key 'eshell-mode "<ret>" 'eshell-send-input)
(define-key 'eshell-mode "<tab>" 'completion-at-point)
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
(menu-bar-add '("Eshell" "Send Password...") 'eshell-send-invisible nil :mode 'eshell-mode)
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
