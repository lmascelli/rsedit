;;; dracula --- Zeno Rocha's purple-forward dark theme.
;;;
;;; A violet-grey page (#282a36) with six pastel accents, every one of them
;;; pale enough to sit on it without glare. Where Monokai is saturated and
;;; Zenburn desaturated, this is light: the accents are high-value rather
;;; than high-chroma.
;;;
;;; The comment colour is the unusual decision -- #6272a4, a blue, rather than
;;; a grey. Comments read as a different *kind* of text rather than as quieter
;;; text.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'dracula-theme 'faces
     '(;; The page itself
       (default             "#f8f8f2" "#282a36" nil)
       (region              nil       "#44475a" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#6272a4" nil       nil)
       (line-number-current "#ffffff" nil       ("bold"))
       (mode-line           "#f8f8f2" "#44475a" ("bold"))
       (mode-line-inactive  "#6272a4" "#343746" nil)
       (window-separator    "#44475a" nil       nil)

       ;; The grammar
       (keyword             "#ff79c6" nil       nil)
       (type                "#8be9fd" nil       nil)
       (function            "#50fa7b" nil       nil)
       (builtin             "#bd93f9" nil       nil)
       (string              "#f1fa8c" nil       nil)
       (comment             "#6272a4" nil       ("italic"))
       (doc-comment         "#8f9bc4" nil       ("italic"))
       (attribute           "#bd93f9" nil       nil)
       (preprocessor        "#ffb86c" nil       nil)
       (error               "#ff5555" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#f8f8f2" "#ffb86c" ("bold"))
       (results-here        nil       "#44475a" nil)
       (results-target      nil       "#343746" nil)
       (preview             "#6272a4" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#ffffff" nil       ("bold"))
       (manpage-underline   "#bd93f9" nil       ("underline"))))
