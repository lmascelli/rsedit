;;; gruvbox-light --- morhetz's retro groove: warm, low-contrast, light.
;;;
;;; The light side of gruvbox, which is not the dark side inverted: it uses the
;;; *faded* accents (#9d0006 rather than #fb4934) because a saturated red that
;;; reads well on brown-black is unreadable on cream.
;;;
;;; That asymmetry is the honest part of the design, and the reason both
;;; variants ship as separate files rather than one with a flag.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'gruvbox-light-theme 'faces
     '(;; The page itself
       (default             "#3c3836" "#fbf1c7" nil)
       (region              nil       "#d5c4a1" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#928374" nil       nil)
       (line-number-current "#282828" nil       ("bold"))
       (mode-line           "#3c3836" "#ebdbb2" ("bold"))
       (mode-line-inactive  "#928374" "#ebdbb2" nil)
       (window-separator    "#bdae93" nil       nil)

       ;; The grammar
       (keyword             "#9d0006" nil       ("bold"))
       (type                "#b57614" nil       nil)
       (function            "#79740e" nil       nil)
       (builtin             "#427b58" nil       nil)
       (string              "#79740e" nil       nil)
       (comment             "#928374" nil       ("italic"))
       (doc-comment         "#7c6f64" nil       ("italic"))
       (attribute           "#8f3f71" nil       nil)
       (preprocessor        "#af3a03" nil       nil)
       (error               "#9d0006" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#3c3836" "#b57614" ("bold"))
       (results-here        nil       "#d5c4a1" nil)
       (results-target      nil       "#ebdbb2" nil)
       (preview             "#928374" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#282828" nil       ("bold"))
       (manpage-underline   "#427b58" nil       ("underline"))))
