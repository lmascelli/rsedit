;;; gruvbox-dark --- morhetz's retro groove: warm, low-contrast, dark.
;;;
;;; Built the other way round from Solarized: the backgrounds are warm brown-grey
;;; rather than neutral, and every accent is offered in a faded and a bright
;;; version. The bright ones are used here, on bg0 -- the pairing the author
;;; calls `dark medium'.
;;;
;;; The warmth is the point. A long session on #282828 is easier than on a
;;; neutral #1e1e1e at the same lightness, which is a claim about eyes rather
;;; than about colour theory.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'gruvbox-dark-theme 'faces
     '(;; The page itself
       (default             "#ebdbb2" "#282828" nil)
       (region              nil       "#504945" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#928374" nil       nil)
       (line-number-current "#fbf1c7" nil       ("bold"))
       (mode-line           "#ebdbb2" "#3c3836" ("bold"))
       (mode-line-inactive  "#928374" "#3c3836" nil)
       (window-separator    "#665c54" nil       nil)

       ;; The grammar
       (keyword             "#fb4934" nil       ("bold"))
       (type                "#fabd2f" nil       nil)
       (function            "#b8bb26" nil       nil)
       (builtin             "#8ec07c" nil       nil)
       (string              "#b8bb26" nil       nil)
       (comment             "#928374" nil       ("italic"))
       (doc-comment         "#a89984" nil       ("italic"))
       (attribute           "#d3869b" nil       nil)
       (preprocessor        "#fe8019" nil       nil)
       (error               "#fb4934" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#ebdbb2" "#d79921" ("bold"))
       (results-here        nil       "#504945" nil)
       (results-target      nil       "#3c3836" nil)
       (preview             "#928374" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#fbf1c7" nil       ("bold"))
       (manpage-underline   "#8ec07c" nil       ("underline"))))
