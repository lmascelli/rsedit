;;; recent-files --- the files opened before, to open again.
;;;
;;; M-g r asks for one of the files `recent-files' lists, completing by any
;;; part of its path, and opens it. User manual: the `session' page.
;;; Design: `doc/architettura-editor.typ, <mod-recent-files>`.

(require 'commands)
(require 'debug)

(defun find-recent-file--candidates (input)
  "The recent files matching INPUT, best first."
  (fuzzy-filter input (recent-files)))

(defcommand find-recent-file () nil
  "Ask for one of the files `recent-files' lists, completing by any part of
its path, and open it."
  (if (null (recent-files))
      (message "No recent files")
      (minibuffer-read "Recent file:"
                       (lambda (path)
                         (if (string= path "")
                             (message "No file chosen")
                             (find-file-async path)))
                       'find-recent-file--candidates
                       nil)))

(define-key nil "M-g r" 'find-recent-file)

(log "recent-files loaded")
(provide 'recent-files)
