;;; lsp --- language servers: what they say about the files being edited.
;;;
;;; lsp-mode, on in a buffer visiting a file, starts the server `lsp-servers'
;;; gives for its major mode at the file's root, keeps the server's copy of
;;; the text in step with the buffer, reports what the server says as
;;; findings, goes to the definitions it gives, takes the code actions it
;;; offers, and completes with its candidates. User manual: the `lsp' page.
;;; Design: `doc/architettura-editor.typ, <mod-lsp>`.

(require 'commands)
(require 'debug)
(require 'project)

;; ---------------------------------------------------------------------------
;; Settings
;; ---------------------------------------------------------------------------

(defvar lsp-servers
  '((rust-mode "rust-analyzer" ("rust-analyzer"))
    (c-mode "clangd" ("clangd"))
    (cpp-mode "clangd" ("clangd"))
    (python-mode "pylsp" ("pylsp")))
  "The language servers, an entry for each major mode:
(MODE NAME (PROGRAM ARGS...) . PROPERTIES). PROPERTIES may give :language, the
language's name for the server, by default MODE without -mode;
:initialization-options, a map sent as it is when the server starts;
:settings, a map from sections to the settings the server asks for; and
:root, a function used for this entry in place of `lsp-root-function'. Of two
entries for one mode the first wins.")

(defvar lsp-root-function 'project-root
  "The function that gives the directory a server runs in, called with the
file of the buffer: a directory, nil for the file's own, or the symbol timeout
to be asked again after the next command.")

(defvar lsp-request-timeout 10
  "How many seconds a question to a server waits for its answer before it is
dropped, with a message.")

(defvar lsp-restart-limit 3
  "How many times in three minutes a server that falls is started again by
itself. Past that it stays down until `lsp-restart'.")

(defvar lsp-log-limit 200000
  "How many characters the log buffer of a server keeps; past that it is cut
from its start.")

;; ---------------------------------------------------------------------------
;; State
;; ---------------------------------------------------------------------------
;;
;; A server is its process name, "lsp NAME ROOT", which is also the producer of
;; its findings; its fields are properties of that name, read with `get' and
;; written with `put'. A document is a buffer: its version and the tick it was
;; last sent at are kept with the buffer, and the server lists its documents
;; as (BUFFER URI FILE).

(defvar lsp--servers nil
  "The names of the servers started in this run, running or not.")

(defvar lsp--waiting nil
  "The buffers whose root has not been found yet, the disk being slow.")

(defvar lsp--refused nil
  "Why lsp-mode last turned itself off, to be said again once its command has
spoken; nil when it did not.")

(defvar lsp--now 0
  "The milliseconds the clock has counted, while it ran.")

(defvar lsp--clock-running nil
  "t while the clock job runs.")

(defvar lsp--queue-limit 5000
  "How many messages may wait for a server to read them before it is taken for
one that no longer reads, and stopped.")

(defvar lsp--stop-wait 2000
  "How many milliseconds a server asked to stop has before it is stopped.")

(defvar lsp--restart-window 180000
  "The milliseconds within which falls count against `lsp-restart-limit'.")

;; ---------------------------------------------------------------------------
;; Small helpers
;; ---------------------------------------------------------------------------

(defun lsp--prop (plist key)
  "The value after KEY in the property list PLIST, or nil."
  (let ((found nil))
    (while (and plist (not found))
      (if (eq (car plist) key)
          (setq found (cdr plist))
          (setq plist (cdr (cdr plist)))))
    (car found)))

(defun lsp--true-p (value)
  "t when the JSON VALUE is neither absent, false nor null."
  (and value (not (eq value 'false)) (not (eq value 'null))))

(defun lsp--list (vector)
  "The elements of the JSON array VECTOR as a list; nil for anything else."
  (let ((items nil) (i (if (vectorp vector) (length vector) 0)))
    (while (> i 0)
      (setq i (- i 1))
      (setq items (cons (aref vector i) items)))
    items))

(defun lsp--without (item list)
  "LIST without the elements `equal' to ITEM."
  (let ((kept nil))
    (dolist (each list)
      (unless (equal each item) (setq kept (cons each kept))))
    (reverse kept)))

(defun lsp--label (server)
  "The name SERVER's entry gives it, for messages."
  (nth 1 (get server 'entry)))

(defun lsp--option (server key)
  "The value of the property KEY in SERVER's entry of `lsp-servers'."
  (lsp--prop (nthcdr 3 (get server 'entry)) key))

(defun lsp--entry (mode)
  "The first entry of `lsp-servers' for the major mode MODE, or nil."
  (let ((found nil) (entries lsp-servers))
    (while (and entries (not found))
      (if (eq (car (car entries)) mode)
          (setq found (car entries)))
      (setq entries (cdr entries)))
    found))

(defun lsp--language (entry)
  "The language ENTRY names for its mode's files."
  (or (lsp--prop (nthcdr 3 entry) :language)
      (let ((mode (symbol-name (car entry))))
        (if (string-suffix-p "-mode" mode)
            (substring mode 0 (- (length mode) 5))
            mode))))

;; ---------------------------------------------------------------------------
;; File names and URIs
;; ---------------------------------------------------------------------------

(defvar lsp--uri-escapes
  '((" " . "%20") ("%" . "%25") ("#" . "%23") ("?" . "%3F") ("\"" . "%22")
    ("<" . "%3C") (">" . "%3E") ("[" . "%5B") ("]" . "%5D") ("{" . "%7B")
    ("}" . "%7D") ("|" . "%7C") ("^" . "%5E") ("`" . "%60") ("\\" . "%5C"))
  "The characters of a file name a URI writes escaped, with their escapes.")

(defun lsp--uri (file)
  "The file URI of the absolute FILE."
  (let* ((pieces nil) (i 0) (count (length file))
         ;; C:\x is file:///C:/x.
         (drive (and (> count 1) (string= (substring file 1 2) ":"))))
    (while (< i count)
      (let* ((char (substring file i (+ i 1)))
             (escape (assoc char lsp--uri-escapes)))
        (setq pieces (cons (cond ((and drive (string= char "\\")) "/")
                                 (escape (cdr escape))
                                 (t char))
                           pieces)))
      (setq i (+ i 1)))
    (concat "file://" (if (string-prefix-p "/" file) "" "/")
            (apply 'concat (reverse pieces)))))

(defun lsp--hex-digit (char)
  "The value of the hexadecimal digit CHAR, or nil."
  (let ((digits "0123456789abcdef") (char (downcase char)) (i 0) (found nil))
    (while (and (< i 16) (not found))
      (if (string= (substring digits i (+ i 1)) char) (setq found i))
      (setq i (+ i 1)))
    found))

(defun lsp--hex4 (number)
  "NUMBER, below 65536, as four hexadecimal digits."
  (let ((digits "0123456789abcdef") (out "") (left 4))
    (while (> left 0)
      (let ((digit (mod number 16)))
        (setq out (concat (substring digits digit (+ digit 1)) out))
        (setq number (floor (/ number 16)))
        (setq left (- left 1))))
    out))

(defun lsp--escape (code)
  "The JSON escape of the character whose code is CODE."
  (if (< code 65536)
      (concat "\\u" (lsp--hex4 code))
      (let ((above (- code 65536)))
        (concat "\\u" (lsp--hex4 (+ 55296 (floor (/ above 1024))))
                "\\u" (lsp--hex4 (+ 56320 (mod above 1024)))))))

(defun lsp--utf-8 (bytes)
  "The string the UTF-8 BYTES, a list of numbers, spell."
  (let ((escapes nil))
    (while bytes
      (let* ((byte (car bytes))
             (more (cond ((< byte 128) 0) ((< byte 224) 1) ((< byte 240) 2) (t 3)))
             (code (cond ((< byte 128) byte) ((< byte 224) (- byte 192))
                         ((< byte 240) (- byte 224)) (t (- byte 240)))))
        (setq bytes (cdr bytes))
        (while (and (> more 0) bytes)
          (setq code (+ (* code 64) (- (car bytes) 128)))
          (setq bytes (cdr bytes))
          (setq more (- more 1)))
        (setq escapes (cons (lsp--escape (max code 0)) escapes))))
    ;; The interpreter has no function from a code to a character; JSON does.
    (json-parse-string (concat "\"" (apply 'concat (reverse escapes)) "\""))))

(defun lsp--file (uri)
  "The file the file URI names, its escapes read; nil for another URI."
  (when (and (stringp uri) (string-prefix-p "file://" uri))
    (let* ((path (substring uri 7)) (count (length path)) (i 0)
           (pieces nil) (bytes nil))
      (while (< i count)
        (let* ((char (substring path i (+ i 1)))
               (high (and (string= char "%") (< (+ i 2) count)
                          (lsp--hex-digit (substring path (+ i 1) (+ i 2)))))
               (low (and high (lsp--hex-digit (substring path (+ i 2) (+ i 3))))))
          (if low
              (progn (setq bytes (cons (+ (* high 16) low) bytes))
                     (setq i (+ i 3)))
              (progn (when bytes
                       (setq pieces (cons (lsp--utf-8 (reverse bytes)) pieces))
                       (setq bytes nil))
                     (setq pieces (cons char pieces))
                     (setq i (+ i 1))))))
      (if bytes (setq pieces (cons (lsp--utf-8 (reverse bytes)) pieces)))
      (let ((file (apply 'concat (reverse pieces))))
        ;; file:///C:/x names C:/x, not /C:/x.
        (if (and (> (length file) 2) (string= (substring file 2 3) ":")
                 (string-prefix-p "/" file))
            (substring file 1)
            file)))))

;; ---------------------------------------------------------------------------
;; The log of a server
;; ---------------------------------------------------------------------------

;; A mode of its own, so that lsp-mode asked for in fundamental-mode does not
;; come on, and refuse, in every log.
(make-mode 'lsp--log-mode)

(defun lsp--log-buffer (server)
  "The name of SERVER's log buffer."
  (concat "*" server "*"))

(defun lsp--log (server text)
  "nil, after adding TEXT as a line to SERVER's log buffer, cut from its start
past `lsp-log-limit'."
  (let ((name (lsp--log-buffer server)))
    (buffer-create name 'lsp--log-mode)
    (with-current-buffer
     name
     (lambda ()
       (goto-char (point-max))
       (insert text "\n")
       (if (> (point-max) lsp-log-limit)
           (delete-region (point-min) (- (point-max) lsp-log-limit)))))
    nil))

(defun lsp--say (server text)
  "nil, after showing TEXT from SERVER in the echo area and its log."
  (lsp--log server text)
  (message "%s: %s" (lsp--label server) text)
  nil)

;; ---------------------------------------------------------------------------
;; Messages going out
;; ---------------------------------------------------------------------------
;;
;; Nothing is ever dropped or sent out of order: a didChange lost would leave
;; the server with another text, and nobody would notice.

(defun lsp--drain (server)
  "t when nothing waits to be written to SERVER, after writing what it takes."
  (let ((queue (get server 'queue)))
    (while (and queue (process-send server (car queue)))
      (setq queue (cdr queue)))
    (put server 'queue queue)
    (when (> (length queue) lsp--queue-limit)
      (put server 'queue nil)
      (lsp--log server "it no longer reads what it is sent: stopped")
      (process-stop server))
    (null queue)))

(defun lsp--send (server message)
  "t when MESSAGE, a map, has been written to SERVER; nil when it waits its
turn, or SERVER is not running."
  (when (memq (get server 'state) '(starting ready stopping))
    (put server 'queue (append (get server 'queue) (list (json-serialize message))))
    (lsp--drain server)))

(defun lsp--notify (server method &optional params)
  "What `lsp--send' says, after sending SERVER the notification METHOD."
  (let ((message (make-map "jsonrpc" "2.0" "method" method)))
    (lsp--send server (if params (map-insert message "params" params) message))))

(defun lsp--reply (server id result)
  "What `lsp--send' says, after answering SERVER's request ID with RESULT."
  (lsp--send server (make-map "jsonrpc" "2.0" "id" id "result" result)))

(defun lsp--request (server method params callback &rest options)
  "The id of the request METHOD with PARAMS sent to SERVER, whose result goes to
CALLBACK with SERVER. OPTIONS: :buffer, the result is wanted only while that
buffer has the same text; :timed, it is dropped after `lsp-request-timeout';
:replace, it takes the place of one of the same METHOD for the same buffer."
  (lsp--flush server)
  (let ((id (+ 1 (or (get server 'last-id) 0)))
        (buffer (lsp--prop options :buffer)))
    (when (lsp--prop options :replace)
      (dolist (old (get server 'requests))
        (when (and (string= (nth 1 old) method) (equal (nth 3 old) buffer))
          (lsp--drop-request server (car old))
          (lsp--notify server "$/cancelRequest" (make-map "id" (car old))))))
    (put server 'last-id id)
    (put server 'requests
         (cons (list id method callback buffer
                     (and buffer (buffer-get 'lsp-version buffer))
                     (and (lsp--prop options :timed) lsp--now))
               (get server 'requests)))
    (let ((message (make-map "jsonrpc" "2.0" "id" id "method" method)))
      (lsp--send server (if params (map-insert message "params" params) message)))
    (if (lsp--prop options :timed) (lsp--clock-start))
    id))

(defun lsp--drop-request (server id)
  "The request ID of SERVER, after taking it from those waiting; nil if none."
  (let ((request (assoc id (get server 'requests))))
    (if request (put server 'requests (lsp--without request (get server 'requests))))
    request))

;; ---------------------------------------------------------------------------
;; Starting and stopping a server
;; ---------------------------------------------------------------------------

(defvar lsp--client-capabilities
  (make-map
   "general" (make-map "positionEncodings" (vector "utf-32" "utf-16"))
   "textDocument" (make-map
                   "synchronization" (make-map "dynamicRegistration" 'false
                                               "willSave" 'false
                                               "willSaveWaitUntil" 'false
                                               "didSave" t)
                   "publishDiagnostics" (make-map "versionSupport" t)
                   "definition" (make-map "linkSupport" t)
                   "references" (make-map)
                   "completion" (make-map "completionItem"
                                          (make-map "snippetSupport" 'false))
                   "codeAction"
                   (make-map "codeActionLiteralSupport"
                             (make-map "codeActionKind"
                                       (make-map "valueSet"
                                                 (vector "" "quickfix" "refactor"
                                                         "refactor.extract" "refactor.inline"
                                                         "refactor.rewrite" "source"
                                                         "source.organizeImports")))
                             "isPreferredSupport" t
                             "disabledSupport" t
                             "dataSupport" t
                             "resolveSupport" (make-map "properties" (vector "edit"))))
   "workspace" (make-map "configuration" t "workspaceFolders" t
                         "applyEdit" t
                         "workspaceEdit" (make-map "documentChanges" t))
   "window" (make-map "workDoneProgress" t))
  "What the client tells a server it can do: only what this module does.")

(defun lsp--initialize-params (server)
  "The parameters of SERVER's initialize request."
  (let* ((root (directory-file-name (get server 'root)))
         (uri (lsp--uri root))
         (options (lsp--option server :initialization-options))
         (params (make-map
                  "processId" 'null
                  "clientInfo" (make-map "name" "rsedit")
                  "rootUri" uri
                  "workspaceFolders"
                  (vector (make-map "uri" uri "name" (file-name-nondirectory root)))
                  "capabilities" lsp--client-capabilities)))
    (if options (map-insert params "initializationOptions" options) params)))

(defun lsp--start (server)
  "t when SERVER's program has started and been asked to initialize; nil,
with SERVER down, when it could not start."
  (let* ((command (nth 2 (get server 'entry)))
         (started (condition-case why
                      (with-directory
                       (get server 'root)
                       (lambda ()
                         (process-start server (car command) (cdr command)
                                        'lsp--deliver 'messages)))
                    (error (lsp--say server (format "cannot start: %s" why)) nil))))
    (put server 'queue nil)
    (put server 'requests nil)
    (if (null started)
        (progn (put server 'state 'dead) nil)
        (progn
          (put server 'state 'starting)
          (lsp--log server (format "started in %s" (get server 'root)))
          (lsp--request server "initialize" (lsp--initialize-params server)
                        'lsp--initialized)
          t))))

(defun lsp--unit (encoding)
  "The unit of `finding-set' for the position ENCODING a server chose."
  (cond ((equal encoding "utf-32") 'char)
        ((equal encoding "utf-8") 'utf-8)
        (t 'utf-16)))

(defun lsp--initialized (server result)
  "nil, after making SERVER ready from RESULT, its answer to initialize, and
opening its documents there."
  (let ((capabilities (if (mapp result) (map-elt result "capabilities") nil)))
    (put server 'capabilities (if (mapp capabilities) capabilities (make-map)))
    (put server 'unit (lsp--unit (map-elt (get server 'capabilities) "positionEncoding")))
    (lsp--notify server "initialized" (make-map))
    (put server 'state 'ready)
    (lsp--log server (format "ready, columns in %s" (get server 'unit)))
    (dolist (document (get server 'documents))
      (lsp--open server (car document)))
    nil))

(defun lsp--stop (server)
  "nil, after asking SERVER to stop: shutdown, then exit, then the process
stopped if it has not ended within two seconds."
  (let ((state (get server 'state)))
    (cond ((eq state 'ready)
           (put server 'asked-stop t)
           (put server 'state 'stopping)
           (lsp--request server "shutdown" nil 'lsp--shut)
           (lsp--arm server))
          ((memq state '(starting stopping))
           (put server 'asked-stop t)
           (process-stop server))))
  nil)

(defun lsp--killed ()
  "nil, after sending every ready server shutdown and exit, and closing its
input, without waiting for its answers: the editor is exiting."
  ;; The exit waits a moment for a server whose input is closed, then stops it.
  (dolist (server lsp--servers)
    (when (eq (get server 'state) 'ready)
      (put server 'asked-stop t)
      (put server 'state 'stopping)
      (lsp--request server "shutdown" nil 'lsp--shut)
      (lsp--notify server "exit")
      (process-close-input server)))
  nil)

(defun lsp--arm (server)
  "nil, after giving SERVER `lsp--stop-wait' milliseconds from now to end."
  (put server 'deadline (+ lsp--now lsp--stop-wait))
  (lsp--clock-start)
  nil)

(defun lsp--shut (server result)
  "nil, after telling SERVER, which has answered shutdown, to exit."
  (lsp--notify server "exit")
  (process-close-input server)
  (lsp--arm server))

(defun lsp--recent (times)
  "The TIMES of falls still within `lsp--restart-window'."
  (let ((kept nil))
    (dolist (time times)
      (if (< (- lsp--now time) lsp--restart-window) (setq kept (cons time kept))))
    (reverse kept)))

(defun lsp--forget (server)
  "nil, after forgetting what SERVER's process had: its findings, requests,
raw diagnostics and queue. Its documents wait for the next process."
  (finding-clear server)
  (put server 'requests nil)
  (put server 'queue nil)
  (put server 'raw (make-map))
  (put server 'progress nil)
  (put server 'deadline nil)
  (put server 'capabilities nil)
  nil)

(defun lsp--exited (server status)
  "nil, after SERVER's process has ended as STATUS says: started again when
that was asked, or when it fell and has not fallen too often."
  (let ((asked (get server 'asked-stop)) (again (get server 'restart)))
    (lsp--forget server)
    (put server 'asked-stop nil)
    (put server 'restart nil)
    (put server 'state 'dead)
    (cond (again
           (put server 'crashes nil)
           (lsp--start server))
          (asked
           (lsp--log server (format "stopped: %s" status)))
          (t
           (let ((crashes (cons lsp--now (lsp--recent (get server 'crashes)))))
             (put server 'crashes crashes)
             (lsp--clock-start)
             (if (> (length crashes) lsp-restart-limit)
                 (lsp--say server
                           (format "fell %d times in three minutes, and stays down: M-x lsp-restart"
                                   (length crashes)))
                 (progn
                   (lsp--say server (format "fell (%s), starting it again" status))
                   (lsp--start server)))))))
  nil)

;; ---------------------------------------------------------------------------
;; The clock
;; ---------------------------------------------------------------------------
;;
;; The interpreter has no clock: a `background-call' job that only sleeps
;; counts time while something waits for it -- an answer, a stop, a fall that
;; still counts -- and stops itself when nothing does.

(defun lsp--clock-start ()
  "nil, after starting the clock unless it runs."
  (unless lsp--clock-running
    (setq lsp--clock-running t)
    (background-call 'lsp-clock (fiber (while t (yield '(sleep 250))))
                     'lsp--clock-step))
  nil)

(defun lsp--clock-needed-p ()
  "t while a server waits for the clock."
  (let ((needed nil))
    (dolist (server lsp--servers)
      (if (or (get server 'deadline) (get server 'crashes)
              (let ((timed nil))
                (dolist (request (get server 'requests))
                  (if (nth 5 request) (setq timed t)))
                timed))
          (setq needed t)))
    needed))

(defun lsp--clock-step (job)
  "nil, after a quarter of a second more of the clock JOB: a server past its
time to stop is stopped, a question past `lsp-request-timeout' dropped."
  (setq lsp--now (+ lsp--now 250))
  (dolist (server lsp--servers)
    (let ((deadline (get server 'deadline)))
      (when (and deadline (>= lsp--now deadline))
        (put server 'deadline nil)
        (process-stop server)))
    (dolist (request (get server 'requests))
      (when (and (nth 5 request)
                 (> (- lsp--now (nth 5 request)) (* 1000 lsp-request-timeout)))
        (lsp--drop-request server (car request))
        (lsp--say server (format "no answer to %s" (nth 1 request)))))
    (put server 'crashes (lsp--recent (get server 'crashes))))
  (unless (lsp--clock-needed-p)
    (setq lsp--clock-running nil)
    (stop-worker job))
  nil)

;; ---------------------------------------------------------------------------
;; What the server says
;; ---------------------------------------------------------------------------

(defun lsp--deliver (server kind value)
  "nil, after handling what SERVER's process gave as KIND: a message, a line of
its standard error, or its exit."
  (let ((server (if (symbolp server) (symbol-name server) server)))
    (cond ((eq kind 'output)
           ;; A callback of another module may have written in a document
           ;; since the last command.
           (lsp--flush-all)
           (lsp--handle server value))
          ((eq kind 'error) (lsp--log server value))
          (t (lsp--exited server value))))
  nil)

(defun lsp--handle (server body)
  "nil, after handling BODY, a message from SERVER: an answer, a request or a
notification."
  (let ((message (condition-case nil (json-parse-string body) (error nil))))
    (if (not (mapp message))
        (lsp--log server (concat "unreadable: " body))
        (let ((method (map-elt message "method"))
              (id (map-elt message "id")))
          (if (eq id 'null) (setq id nil))
          (cond ((and method id)
                 (lsp--answer server id method (map-elt message "params")))
                (method (lsp--notified server method (map-elt message "params")))
                (id (lsp--answered server id message))))))
  nil)

(defun lsp--still-asked-p (request)
  "t when what REQUEST was asked on is still there: its buffer lives, with the
text of then."
  (let ((buffer (nth 3 request)))
    (or (null buffer)
        (and (member buffer (all-buffer-names))
             (equal (buffer-get 'lsp-version buffer) (nth 4 request))))))

(defun lsp--answered (server id message)
  "nil, after giving the answer MESSAGE to the request ID of SERVER waits for."
  (let ((request (lsp--drop-request server id)))
    (when request
      (let ((failure (map-elt message "error")))
        (cond ((mapp failure)
               (lsp--log server (format "%s failed: %s" (nth 1 request)
                                        (map-elt failure "message")))
               (if (string= (nth 1 request) "initialize")
                   (progn (lsp--say server "refused to start")
                          (put server 'asked-stop t)
                          (process-stop server))
                   (if (nth 3 request)
                       (message "%s: %s" (lsp--label server) (map-elt failure "message")))))
              ((lsp--still-asked-p request)
               (funcall (nth 2 request) server (map-elt message "result")))))))
  nil)

(defun lsp--setting (settings section)
  "The part of SETTINGS SECTION names, dots going down; null when none."
  (let ((value settings))
    (dolist (part (if (stringp section) (split-string section ".") nil))
      (setq value (if (mapp value) (map-elt value part 'null) 'null)))
    (if (null value) 'null value)))

(defun lsp--answer (server id method params)
  "nil, after answering the request ID of SERVER, for METHOD with PARAMS."
  (cond ((string= method "workspace/configuration")
         (let ((settings (lsp--option server :settings)))
           (lsp--reply server id
                       (apply 'vector
                              (mapcar (lambda (item)
                                        (lsp--setting settings
                                                      (if (mapp item) (map-elt item "section") nil)))
                                      (lsp--list (map-elt params "items")))))))
        ((string= method "workspace/applyEdit")
         (let ((why (lsp--apply-edit server (map-elt params "edit"))))
           (if why (message "%s: %s" (lsp--label server) why))
           (lsp--reply server id (if why
                                     (make-map "applied" 'false "failureReason" why)
                                     (make-map "applied" t)))))
        ((member method '("client/registerCapability" "client/unregisterCapability"
                          "window/workDoneProgress/create"))
         (lsp--reply server id 'null))
        ((string= method "window/showMessageRequest")
         (lsp--say server (map-elt params "message"))
         (lsp--reply server id 'null))
        (t
         (lsp--send server (make-map "jsonrpc" "2.0" "id" id
                                     "error" (make-map "code" -32601
                                                       "message" (concat "unknown method " method))))))
  nil)

(defun lsp--progress (server params)
  "nil, after logging the progress PARAMS of SERVER; its end is also said."
  (let* ((token (map-elt params "token"))
         (value (map-elt params "value"))
         (kind (if (mapp value) (map-elt value "kind") nil))
         (text (if (mapp value) (map-elt value "message") nil))
         (known (assoc token (get server 'progress))))
    (cond ((equal kind "begin")
           (put server 'progress (cons (cons token (map-elt value "title"))
                                       (get server 'progress)))
           (lsp--log server (format "%s..." (map-elt value "title"))))
          ((equal kind "report")
           (if (stringp text) (lsp--log server text)))
          ((equal kind "end")
           (put server 'progress (lsp--without known (get server 'progress)))
           (lsp--say server (format "%s done" (if known (cdr known) "work"))))))
  nil)

(defun lsp--notified (server method params)
  "nil, after handling SERVER's notification METHOD with PARAMS; those not
known are let by."
  (cond ((string= method "textDocument/publishDiagnostics")
         (lsp--diagnostics server params))
        ((string= method "window/showMessage")
         (lsp--say server (map-elt params "message")))
        ((string= method "window/logMessage")
         (lsp--log server (map-elt params "message")))
        ((string= method "$/progress")
         (lsp--progress server params)))
  nil)

;; ---------------------------------------------------------------------------
;; The findings
;; ---------------------------------------------------------------------------

(defun lsp--severity (number)
  "The severity of a finding for a diagnostic's NUMBER; error when it has none."
  (cond ((equal number 2) 'warning)
        ((equal number 3) 'info)
        ((equal number 4) 'hint)
        (t 'error)))

(defun lsp--position (position unit)
  "The offset in the current buffer of the LSP POSITION, its columns in UNIT."
  (line-column-to-position (+ 1 (map-elt position "line"))
                           (map-elt position "character") unit))

(defun lsp--position-at (offset unit)
  "The LSP position of OFFSET in the current buffer, its column in UNIT."
  (let ((place (position-to-line-column offset unit)))
    (make-map "line" (- (car place) 1) "character" (nth 1 place))))

(defun lsp--place (position)
  "The (LINE . COLUMN) of the LSP POSITION for a finding, its column as the
server counts it."
  (cons (+ 1 (map-elt position "line")) (map-elt position "character")))

(defun lsp--finding (diagnostic)
  "The finding DIAGNOSTIC makes."
  (let* ((range (map-elt diagnostic "range"))
         (code (map-elt diagnostic "code"))
         (message (map-elt diagnostic "message"))
         (finding (list :start (lsp--place (map-elt range "start"))
                        :end (lsp--place (map-elt range "end"))
                        :severity (lsp--severity (map-elt diagnostic "severity"))
                        :message (if (stringp message) message ""))))
    (if (or (stringp code) (numberp code))
        (append finding (list :code (if (stringp code) code (number-to-string code))))
        finding)))

(defun lsp--document (server file)
  "SERVER's document (BUFFER URI FILE) for FILE, or nil."
  (let ((found nil))
    (dolist (document (get server 'documents))
      (if (equal (nth 2 document) file) (setq found document)))
    found))

(defun lsp--diagnostics (server params)
  "nil, after making SERVER's findings in a file what PARAMS, a
publishDiagnostics, says; one about a text that is no longer there is dropped."
  (let* ((file (lsp--file (map-elt params "uri")))
         (version (map-elt params "version"))
         (document (and file (lsp--document server file)))
         (buffer (car document))
         (items (lsp--list (map-elt params "diagnostics")))
         (unit (get server 'unit)))
    (cond ((null file) nil)
          ((and (numberp version) buffer
                (not (equal version (buffer-get 'lsp-version buffer))))
           (lsp--log server (format "dropped the findings of version %d of %s"
                                    version file)))
          (t
           (put server 'raw (if items
                                (map-insert (or (get server 'raw) (make-map)) file
                                            (map-elt params "diagnostics"))
                                (map-delete (or (get server 'raw) (make-map)) file)))
           (condition-case why
               (finding-set server file (mapcar 'lsp--finding items) unit)
             (error (lsp--log server (format "findings refused: %s" why)))))))
  nil)

;; ---------------------------------------------------------------------------
;; The documents
;; ---------------------------------------------------------------------------

(defun lsp--sync-kind (server)
  "How SERVER wants changes: 0 not at all, 1 or 2 as text."
  (let ((sync (map-elt (or (get server 'capabilities) (make-map)) "textDocumentSync")))
    (cond ((numberp sync) sync)
          ((mapp sync) (map-elt sync "change" 0))
          (t 0))))

(defun lsp--open (server buffer)
  "nil, after sending SERVER the whole text of BUFFER as version 1."
  (let* ((document (assoc buffer (get server 'documents)))
         (read (with-current-buffer
                buffer
                (lambda () (list (buffer-string) (buffer-modified-tick))))))
    (buffer-put 'lsp-version 1 buffer)
    (buffer-put 'lsp-tick (nth 1 read) buffer)
    (lsp--notify server "textDocument/didOpen"
                 (make-map "textDocument"
                           (make-map "uri" (nth 1 document)
                                     "languageId" (buffer-get 'lsp-language buffer)
                                     "version" 1
                                     "text" (car read)))))
  nil)

(defun lsp--changed (server buffer tick)
  "nil, after sending SERVER the whole text of BUFFER, changed at TICK, as its
next version."
  (buffer-put 'lsp-tick tick buffer)
  (when (> (lsp--sync-kind server) 0)
    (let ((version (+ 1 (buffer-get 'lsp-version buffer))))
      (buffer-put 'lsp-version version buffer)
      (lsp--notify server "textDocument/didChange"
                   (make-map "textDocument"
                             (make-map "uri" (buffer-get 'lsp-uri buffer)
                                       "version" version)
                             "contentChanges"
                             (vector (make-map "text"
                                               (with-current-buffer
                                                buffer
                                                (lambda () (buffer-string)))))))))
  nil)

(defun lsp--flush (server)
  "nil, after sending SERVER every change its documents have had since it last
heard of them."
  (lsp--drain server)
  (when (eq (get server 'state) 'ready)
    (dolist (document (get server 'documents))
      (let* ((buffer (car document))
             (tick (buffer-modified-tick buffer)))
        (if (and tick (not (equal tick (buffer-get 'lsp-tick buffer))))
            (lsp--changed server buffer tick)))))
  nil)

(defun lsp--flush-all ()
  "nil, after `lsp--flush' for every server."
  (dolist (server lsp--servers) (lsp--flush server))
  nil)

(defun lsp--server-for (file entry)
  "The server for FILE under ENTRY, made and started if need be; timeout when
its root is not known yet."
  (let* ((function (or (lsp--prop (nthcdr 3 entry) :root) lsp-root-function))
         (root (funcall function file)))
    (cond ((eq root 'timeout) 'timeout)
          (t
           (let* ((root (if (stringp root) root (file-name-directory file)))
                  (server (concat "lsp " (nth 1 entry) " " root)))
             (unless (member server lsp--servers)
               (setq lsp--servers (append lsp--servers (list server)))
               (put server 'entry entry)
               (put server 'root root)
               (put server 'documents nil)
               (put server 'crashes nil)
               (lsp--forget server)
               (lsp--start server))
             server)))))

(defun lsp--attach ()
  "nil, after making the current buffer a document of its server; or after
turning lsp-mode off where it cannot be one."
  (let* ((buffer (current-buffer))
         (file (buffer-file-name))
         (entry (lsp--entry (major-mode))))
    (lsp--detach buffer)
    (cond ((null file)
           (setq lsp--refused (format "lsp-mode: %s visits no file" buffer))
           (minor-mode-off 'lsp-mode)
           (message "%s" lsp--refused))
          ((null entry)
           (setq lsp--refused (format "lsp-mode: no server for %s in lsp-servers"
                                      (major-mode)))
           (minor-mode-off 'lsp-mode)
           (message "%s" lsp--refused))
          (t
           (let ((server (lsp--server-for file entry)))
             (if (eq server 'timeout)
                 (unless (member buffer lsp--waiting)
                   (setq lsp--waiting (cons buffer lsp--waiting)))
                 (let ((uri (lsp--uri file)))
                   (buffer-put 'lsp-server server)
                   (buffer-put 'lsp-uri uri)
                   (buffer-put 'lsp-language (lsp--language entry))
                   (buffer-put 'lsp-version 0)
                   (put server 'documents
                        (append (get server 'documents) (list (list buffer uri file))))
                   (cond ((eq (get server 'state) 'ready) (lsp--open server buffer))
                         ((eq (get server 'state) 'dead)
                          (message "%s is down: M-x lsp-restart" (lsp--label server))))))))))
  nil)

(defun lsp--close (server document)
  "nil, after taking DOCUMENT from SERVER's, and telling SERVER when it has it."
  (put server 'documents (lsp--without document (get server 'documents)))
  (if (eq (get server 'state) 'ready)
      (lsp--notify server "textDocument/didClose"
                   (make-map "textDocument" (make-map "uri" (nth 1 document)))))
  nil)

(defun lsp--detach (buffer)
  "nil, after BUFFER is no longer a document, nor waiting to be one."
  (setq lsp--waiting (lsp--without buffer lsp--waiting))
  (let* ((server (buffer-get 'lsp-server buffer))
         (document (and server (assoc buffer (get server 'documents)))))
    (when server
      (lsp--flush server)
      (if document (lsp--close server document))
      (dolist (key '(lsp-server lsp-uri lsp-language lsp-version lsp-tick))
        (buffer-put key nil buffer))))
  nil)

(defun lsp--toggled ()
  "nil, after making the current buffer a document, or not, as lsp-mode is now."
  (setq lsp--refused nil)
  (if (minor-mode-p 'lsp-mode)
      (lsp--attach)
      (lsp--detach (current-buffer)))
  nil)

(defun lsp--renamed ()
  "nil, after the current buffer, written to another file, has left the
server of its old one, and opened the new one where lsp-mode is on."
  (when (or (buffer-get 'lsp-server) (member (current-buffer) lsp--waiting))
    (lsp--detach (current-buffer))
    (if (minor-mode-p 'lsp-mode) (lsp--attach)))
  nil)

(defun lsp--saved ()
  "nil, after telling the server of the current buffer that its file was
saved, when the server wants to know."
  (let* ((server (buffer-get 'lsp-server))
         (sync (and server (eq (get server 'state) 'ready)
                    (map-elt (get server 'capabilities) "textDocumentSync")))
         (save (and (mapp sync) (map-elt sync "save"))))
    (when (lsp--true-p save)
      (lsp--flush server)
      (let ((document (make-map "textDocument"
                                (make-map "uri" (buffer-get 'lsp-uri)))))
        (lsp--notify server "textDocument/didSave"
                     (if (and (mapp save) (lsp--true-p (map-elt save "includeText")))
                         (map-insert document "text" (buffer-string))
                         document)))))
  nil)

(defun lsp--closed ()
  "nil, after closing the document of the buffer `closed-buffer-name'."
  (setq lsp--waiting (lsp--without closed-buffer-name lsp--waiting))
  (dolist (server lsp--servers)
    (dolist (document (get server 'documents))
      (if (equal (car document) closed-buffer-name)
          (lsp--close server document))))
  nil)

(defun lsp--after-command ()
  "nil, after what the last command did has reached the servers: changes sent,
buffers waiting for a root asked for it again, documents whose buffer lost
lsp-mode with its major mode closed."
  (dolist (buffer lsp--waiting)
    (if (member buffer (all-buffer-names))
        (with-current-buffer buffer (lambda () (lsp--attach)))
        (setq lsp--waiting (lsp--without buffer lsp--waiting))))
  (dolist (server lsp--servers)
    (dolist (document (get server 'documents))
      (unless (minor-mode-p 'lsp-mode (car document))
        (lsp--detach (car document)))))
  (lsp--flush-all))

;; ---------------------------------------------------------------------------
;; Going to a definition
;; ---------------------------------------------------------------------------

(make-mode 'lsp-locations-mode)

(defvar lsp--places nil
  "Where `lsp-find-definition' went from, the latest first: (BUFFER . POINT).")

(defconst lsp--locations-buffer "*lsp locations*"
  "The buffer a list of the places a server gave is shown in.")

(defconst lsp--locations-height 12
  "How many rows the window showing a list of places takes.")

(defun lsp--location (item)
  "(FILE . POSITION) for ITEM, a Location or a LocationLink, POSITION the LSP
position of its start; nil when ITEM is neither."
  (let* ((link (and (mapp item) (map-elt item "targetUri")))
         (uri (if link link (and (mapp item) (map-elt item "uri"))))
         (range (cond (link (or (map-elt item "targetSelectionRange")
                                (map-elt item "targetRange")))
                      ((mapp item) (map-elt item "range"))))
         (file (and (stringp uri) (lsp--file uri))))
    (if (and file (mapp range) (mapp (map-elt range "start")))
        (cons file (map-elt range "start"))
        nil)))

(defun lsp--locations (result)
  "The places RESULT, an answer to definition or references, gives, as
`lsp--location' makes them."
  (let ((places nil))
    (dolist (item (cond ((mapp result) (list result))
                        ((vectorp result) (lsp--list result))
                        (t nil)))
      (let ((place (lsp--location item)))
        (if place (setq places (cons place places)))))
    (reverse places)))

(defun lsp--jump (server place from)
  "nil, after going to PLACE, (FILE . POSITION) with POSITION in SERVER's unit,
and remembering FROM, (BUFFER . POINT), for `lsp-go-back'."
  (condition-case why
      (progn
        (find-file (car place))
        (setq lsp--places (cons from lsp--places))
        (goto-char (lsp--position (cdr place) (get server 'unit))))
    (error (message "Cannot open %s: %s" (car place) why)))
  nil)

(defun lsp--entry-of (server place)
  "The entry of a list of results for PLACE, (FILE . POSITION): its column in
characters when FILE is one of SERVER's documents, the server's otherwise."
  (let* ((position (cdr place))
         (line (+ 1 (map-elt position "line")))
         (buffer (car (lsp--document server (car place))))
         (column (if (and buffer (member buffer (all-buffer-names)))
                     (with-current-buffer
                      buffer
                      (lambda ()
                        (nth 1 (position-to-line-column
                                (lsp--position position (get server 'unit))))))
                     (map-elt position "character"))))
    (list "file" (car place) line column)))

(defun lsp--show-places (server what places)
  "nil, after listing PLACES, the (FILE . POSITION) WHAT names, for M-g n to
walk, and showing the list."
  (let ((root (get server 'root))
        (entries (mapcar (lambda (place) (lsp--entry-of server place)) places)))
    (buffer-create lsp--locations-buffer 'lsp-locations-mode)
    (with-current-buffer
     lsp--locations-buffer
     (lambda ()
       (set-buffer-read-only nil)
       (clear-buffer)
       (insert "--- " (number-to-string (length entries)) " " what " ---\n")
       (dolist (entry entries)
         (let ((file (nth 1 entry)))
           (insert (if (string-prefix-p root file) (substring file (length root)) file)
                   ":" (number-to-string (nth 2 entry))
                   ;; From 1, as a compiler writes it.
                   ":" (number-to-string (+ (nth 3 entry) 1)) "\n")))
       ;; Written by the list, not by you: nothing to save.
       (set-buffer-modified-p nil)
       (set-buffer-read-only t)
       (results-put what root entries lsp--locations-buffer)))
    (results-display lsp--locations-buffer lsp--locations-height t)
    (goto-line 1))
  nil)

(defun lsp--found (server result what from)
  "nil, after going to the one place RESULT gives, or listing them when more,
or saying none was found; WHAT names them, FROM is (BUFFER . POINT)."
  (let ((places (lsp--locations result)))
    (cond ((null places) (message "No %s found" what))
          ((null (cdr places)) (lsp--jump server (car places) from))
          (t (lsp--show-places server what places))))
  nil)

(defun lsp--find (method provider what &optional context)
  "nil, after asking the server of the current buffer METHOD at point, a
question its capability PROVIDER says it answers with places; WHAT names
them, CONTEXT is the request's context, if any."
  (let ((server (lsp--server-here))
        (from (cons (current-buffer) (point))))
    (cond ((null server) (message "No language server for this buffer"))
          ((not (eq (get server 'state) 'ready))
           (message "%s is not ready" (lsp--label server)))
          ((not (lsp--true-p (map-elt (get server 'capabilities) provider)))
           (message "%s does not give %s" (lsp--label server) what))
          (t
           (let ((params (make-map "textDocument" (make-map "uri" (buffer-get 'lsp-uri))
                                   "position" (lsp--position-at (point) (get server 'unit)))))
             (lsp--request server method
                           (if context (map-insert params "context" context) params)
                           (lambda (server result) (lsp--found server result what from))
                           :buffer (current-buffer) :timed t :replace t)))))
  nil)

;; ---------------------------------------------------------------------------
;; Code actions
;; ---------------------------------------------------------------------------

(defun lsp--offset (place unit)
  "The offset in the current buffer of PLACE, (LINE . COLUMN), its column in
UNIT."
  (line-column-to-position (car place) (cdr place) unit))

(defun lsp--range-here ()
  "(START . END): the region when it is active, point otherwise."
  (if (use-region-p)
      (cons (region-beginning) (region-end))
      (cons (point) (point))))

(defun lsp--touching (server range)
  "A vector of the diagnostics SERVER sent about the current buffer's file,
as they came, whose findings now touch RANGE, (START . END)."
  (let* ((file (buffer-file-name))
         (raw (lsp--list (map-elt (or (get server 'raw) (make-map)) file)))
         (at 0)
         (chosen nil))
    ;; The store keeps a producer's findings in the order they came: the
    ;; place of one among the server's is the place of its diagnostic.
    (dolist (finding (findings-for-file file))
      (when (equal (symbol-name (lsp--prop finding :producer)) server)
        (let ((unit (lsp--prop finding :unit))
              (diagnostic (nth at raw)))
          (if (and diagnostic
                   (<= (lsp--offset (lsp--prop finding :start) unit) (cdr range))
                   (>= (lsp--offset (lsp--prop finding :end) unit) (car range)))
              (setq chosen (cons diagnostic chosen))))
        (setq at (+ at 1))))
    (apply 'vector (reverse chosen))))

(defun lsp--actions (result)
  "(TITLE . ACTION) for each action of RESULT, an answer to codeAction, that
can be taken: the preferred first, a title met again numbered."
  (let ((preferred nil) (others nil) (titled nil))
    (dolist (item (lsp--list result))
      (when (and (mapp item) (stringp (map-elt item "title"))
                 (not (mapp (map-elt item "disabled"))))
        (if (lsp--true-p (map-elt item "isPreferred"))
            (setq preferred (cons item preferred))
            (setq others (cons item others)))))
    (dolist (item (append (reverse preferred) (reverse others)))
      (let* ((title (map-elt item "title"))
             (count 1))
        (while (assoc (if (= count 1) title (format "%s <%d>" title count)) titled)
          (setq count (+ count 1)))
        (setq titled (cons (cons (if (= count 1) title (format "%s <%d>" title count)) item)
                           titled))))
    (reverse titled)))

(defun lsp--buffer-for (file)
  "The buffer visiting FILE, opened if need be without leaving the current
one; nil when it cannot be opened."
  (let ((found nil))
    (dolist (name (all-buffer-names))
      (if (equal (buffer-file-name name) file) (setq found name)))
    (or found
        (let* ((here (current-buffer))
               (opened (condition-case nil (find-file file) (error nil))))
          (if (member here (all-buffer-names)) (switch-to-buffer here))
          opened))))

(defun lsp--text-edits (document)
  "(FILE VERSION EDITS) for DOCUMENT, an entry of documentChanges, or a string
saying why it cannot be made."
  (let ((target (map-elt document "textDocument")))
    (if (or (stringp (map-elt document "kind")) (not (mapp target)))
        "it creates, renames or deletes files"
        (list (lsp--file (map-elt target "uri"))
              (map-elt target "version")
              (lsp--list (map-elt document "edits"))))))

(defun lsp--workspace-files (edit)
  "(FILE VERSION EDITS) for each file the WorkspaceEdit EDIT changes, or a
string saying why it cannot be made."
  (let ((changes (map-elt edit "documentChanges"))
        (files nil)
        (why nil))
    (if (vectorp changes)
        (dolist (document (lsp--list changes))
          (unless why
            (let ((made (lsp--text-edits document)))
              (if (stringp made) (setq why made) (setq files (cons made files))))))
        (when (mapp (map-elt edit "changes"))
          (dolist (pair (map-pairs (map-elt edit "changes")))
            (setq files (cons (list (lsp--file (car pair)) 'null (lsp--list (cdr pair)))
                              files)))))
    (or why
        (let ((unknown nil))
          (dolist (each files) (if (null (car each)) (setq unknown t)))
          (if unknown "it names a file that is not local" (reverse files))))))

(defun lsp--insert-sorted (edit edits)
  "EDITS, sorted by start, with EDIT, (START END TEXT), after those that start
where it does or before."
  (if (or (null edits) (< (car edit) (car (car edits))))
      (cons edit edits)
      (cons (car edits) (lsp--insert-sorted edit (cdr edits)))))

(defun lsp--replacements (server edits)
  "(START END TEXT) for each TextEdit of EDITS from SERVER, in the current
buffer, sorted by start; a string when two overlap."
  (let ((sorted nil) (unit (get server 'unit)) (why nil) (previous nil))
    (dolist (edit edits)
      (let ((range (map-elt edit "range")))
        (setq sorted (lsp--insert-sorted
                      (list (lsp--position (map-elt range "start") unit)
                            (lsp--position (map-elt range "end") unit)
                            (or (map-elt edit "newText") ""))
                      sorted))))
    (dolist (edit sorted)
      (if (and previous (< (car edit) (nth 1 previous))) (setq why t))
      (setq previous edit))
    (if why (format "two of its changes to %s overlap" (buffer-file-name)) sorted)))

(defun lsp--add-edits (buffer edits buffers)
  "BUFFERS, a list of (BUFFER . EDITS), with EDITS added to those of BUFFER."
  (if (assoc buffer buffers)
      (mapcar (lambda (each)
                (if (equal (car each) buffer) (cons buffer (append (cdr each) edits)) each))
              buffers)
      (append buffers (list (cons buffer edits)))))

(defun lsp--apply-edit (server edit)
  "nil, after making in their buffers the changes of EDIT, a WorkspaceEdit of
SERVER's, all or none; a string saying why none was made."
  (lsp--flush server)
  (let ((files (if (mapp edit) (lsp--workspace-files edit) "it is not an edit"))
        (buffers nil)
        (why nil))
    (if (stringp files) (setq why files))
    ;; Every change is placed on the text as it is before the first is made.
    (dolist (each (if why nil files))
      (unless why
        (let* ((file (car each))
               (version (nth 1 each))
               (buffer (lsp--buffer-for file)))
          (cond ((null buffer) (setq why (format "cannot open %s" file)))
                ((and (numberp version)
                      (not (equal version (buffer-get 'lsp-version buffer))))
                 (setq why (format "%s has changed since" file)))
                ((with-current-buffer buffer (lambda () (buffer-read-only-p)))
                 (setq why (format "%s is read-only" file)))
                (t (setq buffers (lsp--add-edits buffer (nth 2 each) buffers)))))))
    (let ((made nil))
      (dolist (each (if why nil buffers))
        (unless why
          (let ((replacements (with-current-buffer
                               (car each)
                               (lambda () (lsp--replacements server (cdr each))))))
            (if (stringp replacements)
                (setq why replacements)
                (setq made (cons (cons (car each) replacements) made))))))
      ;; `replace-regions' refuses overlaps too, but in one buffer: they are
      ;; looked for here, in every buffer first, so that a refusal changes none.
      (unless why
        (dolist (each (reverse made))
          (with-current-buffer (car each) (lambda () (replace-regions (cdr each)))))
        (lsp--flush-all)))
    why))

(defun lsp--execute (server command buffer)
  "nil, after asking SERVER to run COMMAND, a Command, its arguments as they
came; BUFFER is where it was chosen."
  (let ((params (make-map "command" (map-elt command "command")))
        (arguments (map-elt command "arguments")))
    (lsp--request server "workspace/executeCommand"
                  (if (lsp--true-p arguments) (map-insert params "arguments" arguments) params)
                  (lambda (server result) nil)
                  :buffer buffer :timed t))
  nil)

(defun lsp--perform (server action buffer)
  "nil, after making the changes of ACTION, a CodeAction of SERVER's, then
running its command; BUFFER is where it was chosen."
  (let ((edit (map-elt action "edit"))
        (command (map-elt action "command"))
        (why nil))
    (if (mapp edit) (setq why (lsp--apply-edit server edit)))
    (cond (why (message "%s: %s not made: %s" (lsp--label server)
                        (map-elt action "title") why))
          ((mapp command) (lsp--execute server command buffer))
          ((not (mapp edit))
           (message "%s: %s does nothing" (lsp--label server) (map-elt action "title")))))
  nil)

(defun lsp--take (server action buffer)
  "nil, after taking ACTION, one SERVER offered in BUFFER: asked whole first
when it comes without its changes and SERVER resolves."
  (let ((provider (map-elt (get server 'capabilities) "codeActionProvider")))
    (cond ((stringp (map-elt action "command")) (lsp--execute server action buffer))
          ((and (not (mapp (map-elt action "edit")))
                (mapp provider) (lsp--true-p (map-elt provider "resolveProvider")))
           (lsp--request server "codeAction/resolve" action
                         (lambda (server result)
                           (lsp--perform server (if (mapp result) result action) buffer))
                         :buffer buffer :timed t))
          (t (lsp--perform server action buffer))))
  nil)

(defun lsp--offer (server result buffer)
  "nil, after asking which of the actions of RESULT, SERVER's answer for
BUFFER, to take, and taking it; an empty answer takes the first."
  (let ((actions (lsp--actions result)))
    (if (null actions)
        (message "No code actions here")
        (minibuffer-read "Code action:"
                         (lambda (answer)
                           (let* ((title (if (assoc answer actions)
                                             answer
                                             (car (fuzzy-filter answer (mapcar 'car actions)))))
                                  (action (cdr (assoc title actions))))
                             (if action
                                 (lsp--take server action buffer)
                                 (message "No code action matches %s" answer))))
                         (lambda (input) (fuzzy-filter input (mapcar 'car actions)))
                         nil nil "lsp-code-action")))
  nil)

;; ---------------------------------------------------------------------------
;; Completion
;; ---------------------------------------------------------------------------

(defun lsp--items (result)
  "The CompletionItems of RESULT, an answer to textDocument/completion: a
list of them or a CompletionList."
  (lsp--list (if (mapp result) (map-elt result "items") result)))

(defun lsp--completion-text (item)
  "The text the CompletionItem ITEM puts in place."
  (let ((edit (map-elt item "textEdit")))
    (cond ((and (mapp edit) (stringp (map-elt edit "newText"))) (map-elt edit "newText"))
          ((stringp (map-elt item "insertText")) (map-elt item "insertText"))
          (t (map-elt item "label")))))

(defun lsp--completion-span (server items)
  "(START END) of the text ITEMS, SERVER's completions at point, take the
place of: the range of the first that has one, else the symbol at point."
  (let ((range nil) (unit (get server 'unit)))
    (dolist (item items)
      (let ((edit (map-elt item "textEdit")))
        (if (and (null range) (mapp edit) (mapp (map-elt edit "range")))
            (setq range (map-elt edit "range")))))
    (if range
        (list (lsp--position (map-elt range "start") unit)
              (lsp--position (map-elt range "end") unit))
        (or (bounds-of-thing-at-point 'symbol) (list (point) (point))))))

(defun lsp--completions (server items)
  "(TEXT . EXTRA) for each text ITEMS, SERVER's completions, put in place, the
first item of each kept; EXTRA is what `lsp--replacements' gives for its
additionalTextEdits, nil when it has none."
  (let ((entries nil))
    (dolist (item items)
      (let ((text (lsp--completion-text item))
            (edits (lsp--list (map-elt item "additionalTextEdits"))))
        (unless (or (not (stringp text)) (assoc text entries))
          (setq entries (cons (cons text (if edits (lsp--replacements server edits)))
                              entries)))))
    (reverse entries)))

(defun lsp--complete-extra (server extra span size buffer)
  "nil, after making in BUFFER EXTRA, the (START END TEXT) a chosen completion
brings, worked out when BUFFER had SIZE characters and the completion was to
take the place of SPAN, (START END); a string in EXTRA says why not."
  (let ((why (if (stringp extra) extra))
        (shift (- (with-current-buffer buffer (lambda () (point-max))) size))
        (moved nil))
    ;; Only the text of SPAN has changed since: what follows it has moved.
    (dolist (edit (if why nil extra))
      (cond ((<= (nth 1 edit) (car span)) (setq moved (cons edit moved)))
            ((>= (car edit) (nth 1 span))
             (setq moved (cons (list (+ (car edit) shift) (+ (nth 1 edit) shift) (nth 2 edit))
                               moved)))
            (t (setq why "they overlap the completion"))))
    (cond (why (message "%s: the changes that go with the completion not made: %s"
                        (lsp--label server) why))
          (moved (with-current-buffer buffer (lambda () (replace-regions (reverse moved))))
                 (lsp--flush-all))))
  nil)

(defun lsp--completed (server result buffer at)
  "nil, after offering the completions of RESULT, SERVER's answer for AT in
BUFFER, while point is still there."
  (when (and (equal (current-buffer) buffer) (= (point) at))
    (let ((entries (lsp--completions server (lsp--items result))))
      (if (null entries)
          (message "No completions here")
          (let ((span (lsp--completion-span server (lsp--items result)))
                (size (point-max)))
            (completion-offer (car span) (nth 1 span) (mapcar 'car entries)
                              (lambda (chosen)
                                (lsp--complete-extra server (cdr (assoc chosen entries))
                                                     span size buffer)))))))
  nil)

;; ---------------------------------------------------------------------------
;; The mode and the commands
;; ---------------------------------------------------------------------------

(define-minor-mode lsp-mode "LSP"
  "Keep the file of this buffer open in the language server `lsp-servers'
gives for its major mode, and show what the server says about it as findings."
  ;; Said again: the command's own message has just covered the reason.
  (when lsp--refused
    (message "%s" lsp--refused)
    (setq lsp--refused nil)))

(defun lsp--server-here ()
  "The server of the current buffer, or nil."
  (buffer-get 'lsp-server))

(defcommand lsp-shutdown () nil
  "Stop the language server of this buffer, asking it first. Its findings go,
and its buffers wait for `lsp-restart'."
  (let ((server (lsp--server-here)))
    (cond ((null server) (message "No language server for this buffer"))
          ((eq (get server 'state) 'dead) (message "%s is not running" (lsp--label server)))
          (t (lsp--stop server)
             (message "Stopping %s" (lsp--label server))))))

(defcommand lsp-restart () nil
  "Start the language server of this buffer again: stopped first when it runs,
started at once when it is down, however often it fell."
  (let ((server (lsp--server-here)))
    (cond ((null server) (message "No language server for this buffer"))
          ((eq (get server 'state) 'dead)
           (put server 'crashes nil)
           (if (lsp--start server) (message "Starting %s" (lsp--label server))))
          (t (put server 'restart t)
             (lsp--stop server)
             (message "Restarting %s" (lsp--label server))))))

(defcommand lsp-find-definition () nil
  "Go to the definition of what is at point, as the language server of this
buffer says; with more than one, list them for M-g n. `lsp-go-back' returns."
  (lsp--find "textDocument/definition" "definitionProvider" "definition"))

(defcommand lsp-find-references () nil
  "List the places that use what is at point, its declaration included, as
the language server of this buffer says, for M-g n to walk."
  (lsp--find "textDocument/references" "referencesProvider" "references"
             (make-map "includeDeclaration" t)))

(defcommand lsp-go-back () nil
  "Go back to where the last `lsp-find-definition' went from."
  (let ((from (car lsp--places)))
    (setq lsp--places (cdr lsp--places))
    (cond ((null from) (message "No place to go back to"))
          ((not (member (car from) (all-buffer-names)))
           (message "%s is closed" (car from)))
          (t (switch-to-buffer (car from))
             (goto-char (cdr from))))))

(defcommand lsp-code-action () nil
  "Ask the language server of this buffer what it offers to do at point, or
on the region when it is active, and take the action chosen: its changes
made, one undo for each buffer they touch, and its command run."
  (let ((server (lsp--server-here)))
    (cond ((null server) (message "No language server for this buffer"))
          ((not (eq (get server 'state) 'ready))
           (message "%s is not ready" (lsp--label server)))
          ((not (lsp--true-p (map-elt (get server 'capabilities) "codeActionProvider")))
           (message "%s does not give code actions" (lsp--label server)))
          (t
           (let* ((range (lsp--range-here))
                  (unit (get server 'unit))
                  (buffer (current-buffer)))
             (lsp--request server "textDocument/codeAction"
                           (make-map "textDocument" (make-map "uri" (buffer-get 'lsp-uri))
                                     "range" (make-map "start" (lsp--position-at (car range) unit)
                                                       "end" (lsp--position-at (cdr range) unit))
                                     "context" (make-map "diagnostics"
                                                         (lsp--touching server range)))
                           (lambda (server result) (lsp--offer server result buffer))
                           :buffer buffer :timed t :replace t))))))

(defcommand lsp-complete () nil
  "Complete the text at point with the candidates the language server of
this buffer gives: one is put in place, more are offered to choose from,
and the changes elsewhere that go with the one chosen, an import, are made."
  (let ((server (lsp--server-here)))
    (cond ((null server) (message "No language server for this buffer"))
          ((not (eq (get server 'state) 'ready))
           (message "%s is not ready" (lsp--label server)))
          ((not (lsp--true-p (map-elt (get server 'capabilities) "completionProvider")))
           (message "%s does not complete" (lsp--label server)))
          (t
           (let ((buffer (current-buffer))
                 (at (point)))
             (lsp--request server "textDocument/completion"
                           (make-map "textDocument" (make-map "uri" (buffer-get 'lsp-uri))
                                     "position" (lsp--position-at at (get server 'unit)))
                           (lambda (server result) (lsp--completed server result buffer at))
                           :buffer buffer :timed t :replace t))))))

(defcommand lsp-locations-goto () nil
  "Go to the place on the cursor's line of a list the language server gave."
  ;; Counted, not parsed: a path with a colon reads two ways.
  (let ((index (- (line-number-at-point) 2)))
    (if (and (>= index 0) (< index (or (results-count (current-buffer)) 0)))
        (results-visit index (current-buffer))
        (message "No place on this line"))))

(define-key 'lsp-mode "M-." 'lsp-find-definition)
(define-key 'lsp-mode "M-?" 'lsp-find-references)
(define-key 'lsp-mode "M-," 'lsp-go-back)
(define-key 'lsp-mode "C-c a" 'lsp-code-action)
(define-key 'lsp-mode "C-M-i" 'lsp-complete)
(define-key 'lsp-mode "M-tab" 'lsp-complete)

(define-key 'lsp-locations-mode "<ret>" 'lsp-locations-goto)
(define-key 'lsp-locations-mode "n" 'next-line)
(define-key 'lsp-locations-mode "p" 'previous-line)
(define-key 'lsp-locations-mode "q" 'results-quit)

(add-syntax-rule 'lsp-locations-mode "^--- .* ---$" 'comment)

(add-hook 'lsp-mode "minor-mode-hook" 'lsp--toggled)
(add-hook 'lsp-mode "after-save-hook" 'lsp--saved)
(add-hook nil "after-set-visited-file-name-hook" 'lsp--renamed)
(add-hook nil "after-close-hook" 'lsp--closed)
(add-hook nil "kill-editor-hook" 'lsp--killed)
(add-hook nil "post-command-hook" 'lsp--after-command)

(log "lsp loaded")
(provide 'lsp)
