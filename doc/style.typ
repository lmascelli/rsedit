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

// The size of code blocks. A state rather than a constant so that a document
// can shrink one kind of block (long excerpts, say) without touching the rest.
#let code-block-size = state("code-block-size", 9pt)

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

  // A chapter starts a page. The outline's own title is a level-1 heading
  // too, but not an outlined one, so the outline stays under the title.
  show heading.where(level: 1): it => {
    if it.outlined { pagebreak(weak: true) }
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
  )[#context { set text(size: code-block-size.get()); it }]
  show raw.where(block: false): it => box(
    fill: paper,
    inset: (x: 2.5pt, y: 0pt),
    outset: (y: 2.5pt),
    radius: 2pt,
  )[#set text(size: 9pt); #it]

  show link: it => text(fill: accent, it)

  // In the outline, code in a heading is set as plain monospace: the tinted
  // box that marks it in the text would open a gap before the punctuation.
  show outline.entry: it => {
    show raw: r => text(font: "DejaVu Sans Mono", size: 0.88em, r.text)
    it
  }

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

#let canvas(width: 100%, height: 4cm, code-size: none, body) = block(
  width: width,
  height: height,
  breakable: false,
  {
    // With CODE-SIZE set, inline code in the figure is drawn small and bare --
    // no tinted box -- so that a node naming three types still fits its box.
    show raw.where(block: false): it => if code-size == none { it } else {
      text(size: code-size, font: "DejaVu Sans Mono", it.text)
    }
    body
  },
)

// A labelled box whose centre sits at (x, y).
//
// The box is given a width and a height so that the arrows between boxes can
// be aimed at their edges, but the *text* is measured first and the box grown
// if it would not fit. That is what stops a diagram from quietly breaking when
// a label is reworded: the worst that happens is a box wider than its
// neighbours, rather than text spilling across the page.
#let node(x, y, w, h, label, fill: white, stroke-colour: ink, size: 8.5pt) = context {
  let inner = [#set text(size: size); #set par(justify: false); #label]
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
#let arrow(from, to, colour: ink, dash: none, label: none, label-dx: 0pt, label-dy: -9pt, centered: false) = {
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
    let tag = box(fill: white, inset: (x: 2pt, y: 1pt))[#set text(size: 7.5pt, fill: colour); #label]
    if centered {
      // The label's *centre* goes on the midpoint (plus the offsets), so it
      // sits over the line whatever its length.
      context {
        let m = measure(tag)
        place(
          dx: (x1 + x2) / 2 + label-dx - m.width / 2,
          dy: (y1 + y2) / 2 + label-dy - m.height / 2,
          tag,
        )
      }
    } else {
      place(dx: (x1 + x2) / 2 + label-dx, dy: (y1 + y2) / 2 + label-dy, tag)
    }
  }
}

// An arrow whose label is centred on it, offset by (dx, dy). The usual choice
// for a new figure: the label stays over its line however it is reworded.
#let carrow(from, to, label: none, dx: 0pt, dy: 0pt, colour: ink, dash: none) = arrow(
  from,
  to,
  label: label,
  label-dx: dx,
  label-dy: dy,
  colour: colour,
  dash: dash,
  centered: true,
)

// Free-floating text at a point, for annotating a drawing.
#let note(x, y, body, size: 7.5pt, colour: dim) = place(
  dx: x,
  dy: y,
  box[#set text(size: size, fill: colour); #body],
)

// A note in a sequence diagram: on white, with a thin frame, so that it can
// sit over the lifelines it talks about without being struck through.
#let seq-note(x, y, body, size: 7.5pt, colour: dim) = place(
  dx: x,
  dy: y,
  box(
    fill: white,
    stroke: 0.4pt + dim.lighten(40%),
    inset: (x: 3pt, y: 2.5pt),
    radius: 2pt,
  )[#set text(size: size, fill: colour); #set par(justify: false); #body],
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

// A dashed frame around a group of nodes -- a thread, a compartment, a layer
// -- with its name in the top-left corner. (x, y) is the top-left corner, not
// the centre, because a group is drawn around things already placed.
#let group(x, y, w, h, label, colour: dim) = {
  place(
    dx: x,
    dy: y,
    rect(
      width: w,
      height: h,
      radius: 3pt,
      stroke: (paint: colour, thickness: 0.6pt, dash: "dashed"),
    ),
  )
  place(
    dx: x + 5pt,
    dy: y - 5pt,
    box(fill: white, inset: (x: 2pt, y: 0pt))[#set text(size: 7.5pt, fill: colour, weight: "bold"); #label],
  )
}

// A straight line with no head: the first leg of an arrow that turns a corner,
// or a connection that has no direction.
#let segment(from, to, colour: ink, dash: none) = {
  let (x1, y1) = from
  let (x2, y2) = to
  place(line(start: (x1, y1), end: (x2, y2), stroke: (paint: colour, thickness: 0.7pt, dash: dash)))
}

// ---------------------------------------------------------------------------
// Sequence diagrams
// ---------------------------------------------------------------------------
//
// A participant is a `lane`: a labelled box at the top and a dashed lifeline
// below it. A message is a `msg` between two lanes at a given height; time
// runs down the page. `self-msg` is a call a participant makes to itself.

#let lane(x, top, bottom, label, fill: paper, w: 2.6cm) = {
  node(x, top + 0.35cm, w, 0.7cm, label, fill: fill)
  place(line(
    start: (x, top + 0.7cm),
    end: (x, bottom),
    stroke: (paint: dim, thickness: 0.5pt, dash: "dashed"),
  ))
}

#let msg(from-x, to-x, y, label, colour: ink, dash: none, dy: -6pt) = carrow(
  (from-x, y),
  (to-x, y),
  label: label,
  dy: dy,
  colour: colour,
  dash: dash,
)

#let self-msg(x, y, label, colour: ink, reach: 0.55cm, drop: 0.32cm) = {
  segment((x, y), (x + reach, y), colour: colour)
  segment((x + reach, y), (x + reach, y + drop), colour: colour)
  arrow((x + reach, y + drop), (x + 2pt, y + drop), colour: colour)
  note(x + reach + 4pt, y - 2pt, label, colour: colour)
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

// A reference table: thin rules, a tinted header row, no justification.
// HEADER is a list of cells; the rest are the body, row by row.
#let ref-table(columns: (), header: (), ..cells) = {
  set par(justify: false)
  table(
    columns: columns,
    stroke: 0.4pt + dim.lighten(50%),
    inset: 5pt,
    fill: (_, row) => if row == 0 { paper } else { none },
    table.header(..header.map(h => strong(h))),
    ..cells,
  )
}
