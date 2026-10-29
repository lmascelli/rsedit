;;; shell --- running a command, and reading what it printed.
;;;
;;; M-! runs a command and shows what it prints in a strip along the bottom,
;;; and C-c ! runs the region. User manual: the `shell' page.
;;; Design: `doc/architettura-editor.typ, <mod-shell>`.

(require 'commands)
(require 'debug)

(make-mode 'shell-output-mode)

;; A strip, not half the frame: the output is glanced at while working.
(defvar shell-output-window-height 12
  "How tall the window showing a command's output is, in rows.")

;; The window the newest command's output is shown in: a window id, which may
;; have been closed since, or given to something else.
(setq shell--window nil)

(defun shell--show (name)
  "Show the output buffer NAME in the strip along the bottom."
  ;; One strip for every command, while it still shows one's output; the focus
  ;; stays where M-! was pressed.
  (let ((from (selected-window)))
    (if (and shell--window
             (select-window shell--window)
             (eq (major-mode) 'shell-output-mode))
        (progn (switch-to-buffer name)
               (select-window from))
        (progn (select-window from)
               (setq shell--window
                     (display-buffer-at-bottom name shell-output-window-height))))))

(defcommand shell-command (command) ("sShell command: ")
  "Run COMMAND with the shell, and show what it prints in a strip along the
bottom as it comes; the cursor stays where it is. COMMAND goes to the shell as
written, pipes and redirection included. Its output buffer is its own, and
ends with a line saying how the command ended."
  (let ((name (shell-command-start command)))
    (if name
        (shell--show name)
        ;; `shell-command-start' has said why in the echo area.
        nil)))

(defcommand shell-command-on-region () nil
  "Run the text of the region as a shell command, as `shell-command' does."
  (if (use-region-p)
      (let ((command (buffer-substring (region-beginning) (region-end))))
        (deactivate-mark)
        (shell-command command))
      (message "No region")))

(defcommand shell-command-kill () nil
  "Stop the command whose output is in this buffer, and everything it started:
asked to stop, and made to two seconds later if it has not. The buffer stays,
and ends with --- killed ---."
  (if (stop-worker (current-buffer))
      (message "Stopping %s" (current-buffer))
      (message "Nothing is running in this buffer")))

(define-key nil "M-!" 'shell-command)
(define-key nil "C-c !" 'shell-command-on-region)

(define-key 'shell-output-mode "n" 'next-line)
(define-key 'shell-output-mode "p" 'previous-line)
(define-key 'shell-output-mode "g" 'beginning-of-buffer)
(define-key 'shell-output-mode "G" 'end-of-buffer)
(define-key 'shell-output-mode "q" 'delete-window)
(define-key 'shell-output-mode "C-c C-k" 'shell-command-kill)

;; The exit line, coloured by its number: a failure seen at a glance.
(add-syntax-rule 'shell-output-mode "^--- exited 0 ---$" 'comment)
(add-syntax-rule 'shell-output-mode "^--- exited [1-9][0-9]* ---$" 'error)
(add-syntax-rule 'shell-output-mode "^--- (killed|could not be waited for).*$" 'error)
;; The command being echoed back at the top.
(add-syntax-rule 'shell-output-mode "^\\$ .*$" 'keyword)

(define-face 'error "bright-red" nil '("bold"))

(log "shell loaded")
(provide 'shell)
