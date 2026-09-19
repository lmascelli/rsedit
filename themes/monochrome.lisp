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
       (error       nil nil ("bold" "underline"))

       ;; The editor's own furniture. Reverse video rather than an attribute:
       ;; these are areas rather than words, and an area has to be visible as
       ;; a shape.
       (region             nil nil ("reverse"))
       (mode-line          nil nil ("reverse"))
       (mode-line-inactive nil nil ("bold"))
       (window-separator   nil nil ("bold"))

       ;; The marks other modules make. Reverse, for the same reason.
       (manpage-bold       nil nil ("bold"))
       (manpage-underline  nil nil ("underline"))
       (compilation-here   nil nil ("reverse"))
       (compilation-target nil nil ("reverse"))))
