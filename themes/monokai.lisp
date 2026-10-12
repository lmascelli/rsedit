;;; monokai --- Wimer Hazenberg's high-saturation classic.
;;;
;;; The scheme that made syntax colouring look deliberate rather than
;;; accidental, by way of TextMate in 2006. Its signature is the pink-green
;;; pairing: keywords in #f92672, everything declared in #a6e22e, and a
;;; yellow-olive for strings that keeps them out of the way of both.
;;;
;;; High saturation on a near-black warm ground, which is the opposite bet to
;;; Solarized's and is why the two are usually argued about together.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'monokai-theme 'faces
     '(;; The page itself
       (default             "#f8f8f2" "#272822" nil)
       (region              nil       "#49483e" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#75715e" nil       nil)
       (line-number-current "#ffffff" nil       ("bold"))
       (mode-line           "#f8f8f2" "#49483e" ("bold"))
       (mode-line-inactive  "#75715e" "#3e3d32" nil)
       (window-separator    "#49483e" nil       nil)

       ;; The grammar
       (keyword             "#f92672" nil       nil)
       (type                "#66d9ef" nil       nil)
       (function            "#a6e22e" nil       nil)
       (builtin             "#ae81ff" nil       nil)
       (string              "#e6db74" nil       nil)
       (comment             "#75715e" nil       ("italic"))
       (doc-comment         "#8f908a" nil       ("italic"))
       (attribute           "#ae81ff" nil       nil)
       (preprocessor        "#fd971f" nil       nil)
       (error               "#f92672" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#f8f8f2" "#fd971f" ("bold"))
       (results-here        nil       "#49483e" nil)
       (results-target      nil       "#3e3d32" nil)
       (preview             "#75715e" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#ffffff" nil       ("bold"))
       (manpage-underline   "#ae81ff" nil       ("underline"))))
