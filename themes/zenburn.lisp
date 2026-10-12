;;; zenburn --- Jani Nurminen's low-contrast dark theme.
;;;
;;; Older than most of the others -- Vim, 2003 -- and the first scheme written
;;; against a stated goal: nothing on the screen should be bright. Everything
;;; sits in a narrow band around #3f3f3f, and the accents are desaturated
;;; almost to grey.
;;;
;;; It reads as washed out beside Monokai, which is exactly what it was for.
;;; The faded olive-green comment (#7f9f7f) and the dusty red string (#cc9393)
;;; are the two colours everyone recognises it by.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'zenburn-theme 'faces
     '(;; The page itself
       (default             "#dcdccc" "#3f3f3f" nil)
       (region              nil       "#2f2f2f" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#8f8f8f" nil       nil)
       (line-number-current "#ffffef" nil       ("bold"))
       (mode-line           "#8fb28f" "#2b2b2b" ("bold"))
       (mode-line-inactive  "#8f8f8f" "#4f4f4f" nil)
       (window-separator    "#6f6f6f" nil       nil)

       ;; The grammar
       (keyword             "#f0dfaf" nil       ("bold"))
       (type                "#dfdfbf" nil       nil)
       (function            "#efef8f" nil       nil)
       (builtin             "#dca3a3" nil       nil)
       (string              "#cc9393" nil       nil)
       (comment             "#7f9f7f" nil       ("italic"))
       (doc-comment         "#9fc59f" nil       ("italic"))
       (attribute           "#dfaf8f" nil       nil)
       (preprocessor        "#ffcfaf" nil       nil)
       (error               "#e37170" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#8fb28f" "#dfaf8f" ("bold"))
       (results-here        nil       "#2f2f2f" nil)
       (results-target      nil       "#4f4f4f" nil)
       (preview             "#8f8f8f" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#ffffef" nil       ("bold"))
       (manpage-underline   "#dca3a3" nil       ("underline"))))
