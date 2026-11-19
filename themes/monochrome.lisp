;;; monochrome --- no colour at all, only emphasis.
;;;
;;; Every face is given an attribute and no colour, so the editor reads the
;;; same on a white terminal, a black one, a projector and a printout. It is
;;; also the strongest test of the theme system: a theme must be able to say
;;; \"bold\" where it cannot say \"blue\".
;;;
;;; A theme file is data. Loading it defines a symbol and changes nothing;
;;; `select-theme' is what applies it.

(put 'monochrome-theme 'faces
     ;; Each entry is (FACE FOREGROUND BACKGROUND ATTRIBUTES), which is what
     ;; `set-face' takes -- so a theme is written the way a face is set, and
     ;; there is no second vocabulary to learn.
     '((keyword     nil nil ("bold"))
       (type        nil nil ("underline"))
       (function    nil nil ("bold"))
       (builtin     nil nil ("bold"))
       (comment     nil nil ("italic"))
       (doc-comment nil nil ("italic"))
       (string      nil nil ("italic"))
       (attribute   nil nil ("italic"))
       (preprocessor nil nil ("bold"))
       (error       nil nil ("bold" "underline"))

       ;; The editor's own furniture. Reverse video rather than an attribute:
       ;; these are areas rather than words, and an area has to be visible as
       ;; a shape.
       (region             nil nil ("reverse"))
       ;; Bold, not reverse: the line point is on is where the text is read,
       ;; and the words coloured in it are drawn over it.
       (hl-line            nil nil ("underline"))
       (mode-line          nil nil ("reverse"))
       (mode-line-inactive nil nil ("bold"))
       (window-separator   nil nil ("bold"))

       ;; The menus a terminal draws. Reverse for the bar and the boxes,
       ;; which are areas; the entry the keys are on stands out of them
       ;; by not being reversed.
       (menu               nil nil ("reverse"))
       (menu-selected      nil nil ("bold"))
       (menu-disabled      nil nil ("reverse" "italic"))

       ;; The marks other modules make. Reverse, for the same reason.
       (manpage-bold       nil nil ("bold"))
       (manpage-underline  nil nil ("underline"))
       (results-here       nil nil ("reverse"))
       (results-target     nil nil ("reverse"))
       ;; Findings: the spans a tool reported, and their signs in the gutter
       (finding-error      nil nil ("bold" "underline"))
       (finding-error-sign nil nil ("bold"))
       (finding-warning    nil nil ("underline"))
       (finding-warning-sign nil nil ("bold"))
       (finding-info       nil nil ("underline"))
       (finding-info-sign  nil nil ("bold"))
       (finding-hint       nil nil ("underline"))
       (finding-hint-sign  nil nil ("bold"))
       (lazy-highlight     nil nil ("underline"))
       (show-paren-match   nil nil ("bold" "reverse"))
       (show-paren-mismatch nil nil ("bold" "underline" "reverse"))
       (trailing-whitespace nil nil ("reverse"))
       (preview            nil nil ("italic"))))
