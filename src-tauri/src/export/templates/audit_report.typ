// Wartungsdoku -- compliance/audit-trail report PDF template.
//
// Receives via sys.inputs (see export::pdf::render_audit_report_pdf):
//   customer_name: str
//   generated_at:  str   -- already fully formatted (date/time/zone)
//   rows: array of (
//     at_display:  str   -- already fully formatted (date/time/zone)
//     scope_label: str   -- "Kunde" or a system's name
//     action:      str
//     summary:     str
//   )

#import sys: inputs

#let customer-name = inputs.customer_name
#let generated-at = inputs.generated_at
#let rows = inputs.rows

#set document(title: customer-name + " – Prüfprotokoll")
#set text(size: 10pt, lang: "de", font: "Libertinus Serif")
#set page(paper: "a4", margin: (top: 3.2cm, bottom: 2.8cm, x: 2.5cm))

// -- Cover page ---------------------------------------------------------
#align(center + horizon)[
  #text(24pt, weight: "bold")[Prüfprotokoll]
  #v(1.5em)
  #text(16pt)[#customer-name]
  #v(2em)
  #text(11pt, fill: gray)[Erstellt am #generated-at]
]

#pagebreak()

// -- Header/footer on every content page ---------------------------------
#set page(
  header: [
    #set text(size: 9pt, fill: gray)
    #customer-name -- Prüfprotokoll
    #h(1fr)
    #generated-at
    #v(-0.4em)
    #line(length: 100%, stroke: 0.5pt + gray)
  ],
  footer: context [
    #line(length: 100%, stroke: 0.5pt + gray)
    #v(-0.4em)
    #set text(size: 9pt, fill: gray)
    #align(right)[Seite #counter(page).display()]
  ],
)

// -- Content ---------------------------------------------------------------
#if rows.len() == 0 [
  #text(fill: gray)[Keine protokollierten Änderungen im gewählten Zeitraum.]
] else [
  #table(
    columns: (auto, auto, auto, 1fr),
    stroke: 0.5pt + gray,
    inset: 6pt,
    table.header([*Zeitpunkt*], [*Bereich*], [*Aktion*], [*Zusammenfassung*]),
    ..rows.map(r => (r.at_display, r.scope_label, r.action, r.summary)).flatten()
  )
]
