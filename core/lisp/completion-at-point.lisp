;;; completion-at-point --- the sources C-M-i asks what could go at point.
;;;
;;; The command, which merges what the sources offer and puts the answer in,
;;; is built in; this module is the sources, each of which
;;; `set-completion-functions' can take out on its own. User manual: the
;;; `completion' page.
;;;
;;; A source of names takes its bounds from `bounds-of-thing-at-point', so that
;;; it claims the same text as the others and its candidates are merged with
;;; theirs; only `capf-file-name' claims different text, a whole path.

;; ---------------------------------------------------------------------------
;; Files
;; ---------------------------------------------------------------------------

(defun capf-file-name ()
  "Complete a path when point is in one, over the entries of its directory."
  ;; `file-name-directory' is the test as well as the answer: nil for text with
  ;; no separator, on every platform. An empty or missing directory gives nil,
  ;; not an empty list, which would fix the region and shut the others out.
  (let ((bounds (bounds-of-thing-at-point 'filename)))
    (if bounds
        (let* ((start (nth 0 bounds))
               (end (nth 1 bounds))
               (text (buffer-substring start end))
               (directory (file-name-directory text))
               (entries (if directory (list-dir directory) nil)))
          (if entries
              (list start end
                    (mapcar (lambda (entry) (concat directory entry))
                            entries)))))))

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

(defun capf-symbols ()
  "Complete the names of the functions, macros and variables there are now,
each described as which of the three it is."
  (capf--offer
   (append (mapcar (lambda (name) (cons name "function")) (all-functions))
           (append (mapcar (lambda (name) (cons name "macro")) (all-macros))
                   (mapcar (lambda (name) (cons name "variable")) (all-variables))))))

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
  ;; Consed and reversed rather than appended to, which would copy the list
  ;; for every word of every buffer.
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
;; name offered twice the first description stays. A mode's own sources come
;; before all of these.

(set-completion-functions nil
                          '(capf-file-name
                            capf-mode-keywords
                            capf-symbols
                            capf-buffer-names
                            capf-buffer-words))

(log "End of the completion-at-point.lisp")
