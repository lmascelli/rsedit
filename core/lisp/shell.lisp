;;; shell --- running a command and reading what it said.
;;;
;;; `M-!' asks for a command, runs it, and shows its output in a window beside
;;; what you were doing. The command runs in the background: the editor stays
;;; usable while a build runs, and the output appears as it is produced.
;;;
;;; # What the mode is for
;;;
;;; `shell-output-mode' exists so the transcript can be read without being
;;; typed into, and so `q' means what it means in every other listing here. The
;;; buffer is read-only from the moment it is created -- the worker thread
;;; lifts that for exactly as long as it takes to append a line, under the
;;; buffer's own lock, which is a stronger guarantee than a Lisp module can
;;; make for itself.
;;;
;;; # Several at once
;;;
;;; Each command gets a buffer of its own: `*Shell Output*', then
;;; `*Shell Output*<2>'. Two commands writing into one buffer would interleave
;;; their lines into something neither of them said. The cost is that a long
;;; session leaves buffers behind -- `C-x C-b' and `d' is how they go.

(make-mode 'shell-output-mode)

(defconst shell-output-window-height 12
  "How tall the window showing a command's output is.

A strip across the bottom rather than half the frame: the output is something
you glance at while working, and taking half the screen for `git status' would
make `M-!' a thing you undo afterwards.")

(defun shell--show (name)
  "Show the output buffer called NAME in a strip at the bottom.

Focus does not move. `M-!' is run from somewhere, and that somewhere is where
you want to still be when the output starts arriving."
  (display-buffer-at-bottom name shell-output-window-height))

(defcommand shell-command (command) ("sShell command: ")
  "Run COMMAND with the shell and show its output. Bound to M-!.

Returns straight away -- the command runs in the background and its output
appears as it is produced, ending with a line saying what it exited with. What
you type goes to the shell as written, so pipes and redirection work.

Each run gets its own output buffer, so starting a second command does not
disturb the first."
  (let ((name (shell-command-start command)))
    (if name
        (shell--show name)
        ;; `shell-command-start' has already said why in the echo area; saying
        ;; something else here would paint over it.
        nil)))

(defcommand shell-command-on-region () nil
  "Run the region as a shell command. Bound to C-c !.

For the case where the command is already written down -- in a comment, in a
README, in the transcript of an earlier command -- and retyping it into a
prompt is the only thing standing between reading it and running it."
  (if (use-region-p)
      (let ((command (buffer-substring (region-beginning) (region-end))))
        (deactivate-mark)
        (shell-command command))
      (message "No region")))

(define-key nil "M-!" 'shell-command)
(define-key nil "C-c !" 'shell-command-on-region)

(define-key 'shell-output-mode "n" 'next-line)
(define-key 'shell-output-mode "p" 'previous-line)
(define-key 'shell-output-mode "g" 'beginning-of-buffer)
(define-key 'shell-output-mode "G" 'end-of-buffer)
(define-key 'shell-output-mode "q" 'delete-window)

;; The exit line, so a failure is visible at a glance rather than read. The
;; face is chosen by what the number is, which is the whole of why the trailer
;; is written in a fixed shape.
(add-syntax-rule 'shell-output-mode "^--- exited 0 ---$" 'comment)
(add-syntax-rule 'shell-output-mode "^--- exited [1-9][0-9]* ---$" 'error)
(add-syntax-rule 'shell-output-mode "^--- (killed|could not be waited for).*$" 'error)
;; The command being echoed back at the top.
(add-syntax-rule 'shell-output-mode "^\\$ .*$" 'keyword)

(set-face 'error "bright-red" nil '("bold"))

(log "shell loaded")
