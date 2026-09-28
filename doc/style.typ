// Shared look and drawing helpers for the rsedit documents.
//
// Everything a diagram in these documents needs lives here, so that a diagram
// is a handful of calls with names in them rather than a page of coordinates.
// Change a colour or a box size here and every figure follows.

#let ink = rgb("#1b1b1b")
#let dim = rgb("#6a6a6a")
#let accent = rgb("#1f5fa9")
#let warm = rgb("#a9531f")
#let paper = rgb("#f4f2ee")

#let preamble(title: none, subtitle: none, doc) = {
  set page(
    paper: "a4",
    margin: (x: 2.4cm, y: 2.4cm),
    numbering: "1",
    number-align: center,
  )
  set text(font: ("Libertinus Serif", "DejaVu Serif"), size: 10.5pt, fill: ink)
  set par(justify: true, leading: 0.68em)
  set heading(numbering: "1.1")

  show heading.where(level: 1): it => {
    pagebreak(weak: true)
    block(above: 0pt, below: 1.1em)[
      #set text(size: 17pt, weight: "bold")
      #it
    ]
  }
  show heading.where(level: 2): it => block(above: 1.5em, below: 0.7em)[
    #set text(size: 12.5pt, weight: "bold")
    #it
  ]
  show heading.where(level: 3): it => block(above: 1.2em, below: 0.5em)[
    #set text(size: 11pt, weight: "bold", style: "italic")
    #it
  ]

  show raw.where(block: true): it => block(
    width: 100%,
    fill: paper,
    inset: (x: 9pt, y: 8pt),
    radius: 3pt,
    stroke: 0.4pt + dim.lighten(40%),
  )[#set text(size: 9pt); #it]
  show raw.where(block: false): it => box(
    fill: paper,
    inset: (x: 2.5pt, y: 0pt),
    outset: (y: 2.5pt),
    radius: 2pt,
  )[#set text(size: 9pt); #it]

  show link: it => text(fill: accent, it)

  align(center)[
    #block(inset: (top: 2cm, bottom: 1.2cm))[
      #text(size: 22pt, weight: "bold")[#title]
      #if subtitle != none [
        #linebreak()
        #v(0.4em)
        #text(size: 12pt, fill: dim)[#subtitle]
      ]
    ]
  ]

  doc
}

// A callout for the one sentence a section turns on.
#let key(body) = block(
  width: 100%,
  inset: 9pt,
  radius: 3pt,
  fill: accent.lighten(92%),
  stroke: (left: 2.5pt + accent),
)[#body]

// A warning about something that is refused rather than supported.
#let caution(body) = block(
  width: 100%,
  inset: 9pt,
  radius: 3pt,
  fill: warm.lighten(90%),
  stroke: (left: 2.5pt + warm),
)[#body]

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------
//
// Diagrams are drawn inside a `canvas` of a stated width and height, with
// every element placed at a named point. Points are fractions of the canvas,
// so widening a figure moves everything with it.

#let canvas(width: 100%, height: 4cm, body) = block(
  width: width,
  height: height,
  breakable: false,
  body,
)

// A labelled box whose centre sits at (x, y).
//
// The box is given a width and a height so that the arrows between boxes can
// be aimed at their edges, but the *text* is measured first and the box grown
// if it would not fit. That is what stops a diagram from quietly breaking when
// a label is reworded: the worst that happens is a box wider than its
// neighbours, rather than text spilling across the page.
#let node(x, y, w, h, label, fill: white, stroke-colour: ink, size: 8.5pt) = context {
  let inner = [#set text(size: size); #label]
  let m = measure(box(width: w - 8pt, inner))
  let w = calc.max(w, m.width + 10pt)
  let h = calc.max(h, m.height + 8pt)
  place(
    dx: x - w / 2,
    dy: y - h / 2,
    box(
      width: w,
      height: h,
      fill: fill,
      stroke: 0.7pt + stroke-colour,
      radius: 2.5pt,
      inset: 4pt,
    )[
      #set align(center + horizon)
      #inner
    ],
  )
}

// A straight arrow from one point to another, with the head drawn as a
// triangle so it survives any stroke width.
#let arrow(from, to, colour: ink, dash: none, label: none, label-dx: 0pt, label-dy: -9pt) = {
  let (x1, y1) = from
  let (x2, y2) = to
  place(line(start: (x1, y1), end: (x2, y2), stroke: (paint: colour, thickness: 0.7pt, dash: dash)))
  // The head: a short filled triangle pointing along the line. Computed from
  // the two endpoints so that moving either end keeps it attached.
  let dx = x2 - x1
  let dy = y2 - y1
  let len = calc.sqrt(calc.pow(dx / 1pt, 2) + calc.pow(dy / 1pt, 2)) * 1pt
  let ux = dx / (len / 1pt) / 1pt
  let uy = dy / (len / 1pt) / 1pt
  let head = 5pt
  let wing = 2.4pt
  place(
    polygon(
      fill: colour,
      stroke: none,
      (x2, y2),
      (x2 - ux * head + uy * wing, y2 - uy * head - ux * wing),
      (x2 - ux * head - uy * wing, y2 - uy * head + ux * wing),
    ),
  )
  if label != none {
    place(
      dx: (x1 + x2) / 2 + label-dx,
      dy: (y1 + y2) / 2 + label-dy,
      box(fill: white, inset: (x: 2pt, y: 1pt))[#set text(size: 7.5pt, fill: colour); #label],
    )
  }
}

// Free-floating text at a point, for annotating a drawing.
#let note(x, y, body, size: 7.5pt, colour: dim) = place(
  dx: x,
  dy: y,
  box[#set text(size: size, fill: colour); #body],
)

// One cell of a drawn stack of frames, from the bottom up.
#let frame-stack(x, bottom, w, rows, row-h: 0.62cm, gap: 2pt) = {
  let n = rows.len()
  for (i, row) in rows.enumerate() {
    let y = bottom - (i * (row-h + gap)) - row-h / 2
    node(
      x,
      y,
      w,
      row-h,
      row.at("label"),
      fill: row.at("fill", default: white),
      size: 8pt,
    )
  }
}
