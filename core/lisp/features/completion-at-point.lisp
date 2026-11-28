;;; completion-at-point --- the sources C-M-i asks what could go at point.
;;;
;;; The command, which merges what the sources offer, is built in; this module
;;; is the five sources, their order, and the file names they and other
;;; modules' sources complete. User manual: the `completion' page.
;;; Design: `doc/architettura-editor.typ, <mod-completion-at-point>`.

;; ---------------------------------------------------------------------------
;; Files
;; ---------------------------------------------------------------------------

(defun capf-file-candidates (text &optional here)
  "Return the paths TEXT can be completed to: the entries of TEXT's
directory, each after that directory as TEXT writes it, a subdirectory with a
/ after it. A TEXT with no directory in it is completed in the working
directory when HERE is not nil, and to nothing otherwise.

Example:
  (capf-file-candidates \"src/ma\")   => (\"src/main.rs\" \"src/modes/\" ...)"
  (let ((directory (file-name-directory text)))
    (cond (directory
           (mapcar (lambda (entry) (concat directory entry)) (list-dir directory)))
          (here (list-dir))
          (t nil))))

(defun capf-file-name ()
  "Complete a path when point is in one, over the entries of its directory."
  ;; nil, not an empty list, for text with no directory or an empty one: an
  ;; empty list would fix the region and shut the other sources out.
  (let ((bounds (bounds-of-thing-at-point 'filename)))
    (if bounds
        (let* ((start (nth 0 bounds))
               (end (nth 1 bounds))
               (entries (capf-file-candidates (buffer-substring start end))))
          (if entries
              (list start end entries))))))

;; ---------------------------------------------------------------------------
;; The language of the buffer
;; ---------------------------------------------------------------------------

(defun capf-mode-keywords ()
  "Complete the keywords of the buffer's language: the list its mode keeps
under the property keywords of the mode's symbol."
  (capf--offer (mapcar (lambda (word) (cons word "keyword"))
                       (get (major-mode) 'keywords))))

;; ---------------------------------------------------------------------------
;; The interpreter
;; ---------------------------------------------------------------------------

(defvar capf-symbols-modes '(risp-mode eshell-mode)
  "The major modes Lisp is written in, where `capf-symbols' answers. The REPL
is a risp-mode buffer; eshell asks it only for a line evaluated as Lisp.")

(defun capf-symbols ()
  "Complete the names of the functions, macros and variables there are now,
each described as which of the three it is, in a mode of
`capf-symbols-modes'."
  (if (member (major-mode) capf-symbols-modes)
      (capf--offer
       (append (mapcar (lambda (name) (cons name "function")) (all-functions))
               (append (mapcar (lambda (name) (cons name "macro")) (all-macros))
                       (mapcar (lambda (name) (cons name "variable")) (all-variables)))))))

;; ---------------------------------------------------------------------------
;; Buffers
;; ---------------------------------------------------------------------------

(defun capf-buffer-names ()
  "Complete the name of a buffer."
  (capf--offer (mapcar (lambda (name) (cons name "buffer"))
                       (all-buffer-names))))

(defun capf-buffer-words ()
  "Complete a word written in a buffer, this one's first, each described by
where it was found. The word being typed is not offered."
  ;; Offered back, the half-typed word would be every candidate's common
  ;; start, and with no strip the key would seem to do nothing.
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if bounds
        (let* ((start (nth 0 bounds))
               (end (nth 1 bounds))
               (typed (buffer-substring start end))
               (candidates (capf--words-except typed)))
          (if candidates (list start end candidates))))))

(defun capf--words-except (typed)
  "Every word in every buffer but TYPED, as (WORD . WHERE), this buffer's
first."
  ;; Consed and reversed: appending would copy the list for every word.
  (let ((here (current-buffer))
        (found nil))
    (dolist (word (buffer-words))
      (if (not (string= word typed))
          (setq found (cons (cons word "word") found))))
    (dolist (name (all-buffer-names))
      (if (not (string= name here))
          (dolist (word (buffer-words name))
            (if (not (string= word typed))
                (setq found (cons (cons word name) found))))))
    (reverse found)))

;; ---------------------------------------------------------------------------
;; The shape every source above shares
;; ---------------------------------------------------------------------------

(defun capf--offer (candidates)
  "(START END CANDIDATES) for the symbol at point, or nil when there is none
or CANDIDATES is empty."
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if (and bounds candidates)
        (list (nth 0 bounds) (nth 1 bounds) candidates))))

;; ---------------------------------------------------------------------------
;; The default list
;; ---------------------------------------------------------------------------
;;
;; Most specific first: the first source to answer fixes the region, and of a
;; name offered twice the first description stays. Asked by priority, all
;; below 0, so that a source given none, a mode's own above all, comes first.

(set-completion-functions nil
                          '(capf-file-name
                            capf-mode-keywords
                            capf-symbols
                            capf-buffer-names
                            capf-buffer-words))

(set-completion-priority 'capf-file-name -10)
(set-completion-priority 'capf-mode-keywords -20)
(set-completion-priority 'capf-symbols -30)
(set-completion-priority 'capf-buffer-names -40)
(set-completion-priority 'capf-buffer-words -50)

(log "End of the completion-at-point.lisp")
(provide 'completion-at-point)
