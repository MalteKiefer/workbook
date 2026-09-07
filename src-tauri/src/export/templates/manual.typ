// Wartungsdoku -- PDF-Handbuch-Vorlage.
//
// Empfängt die komplette Eintragsliste über `sys.inputs` (siehe
// `export::pdf::render_manual_pdf`, das dieses Template mit `typst-as-lib`
// kompiliert). Erwartete Struktur von `inputs`:
//
//   customer_name:  str
//   generated_at:   str   -- bereits fertig formatiert (Datum/Zeit/Zone)
//   sections: array of (
//     system_name: str,
//     entries: array of (
//       title: str,
//       category_label: str,
//       performed_at_display: str,
//       late_entry_note: str | none,
//       tags: array of str,
//       body_typst: str    -- bereits von markdown_to_typst::convert() erzeugt,
//                             wird unten per #eval(..., mode: "markup") gerendert
//       image_paths: array of str -- relativ zu data_dir, absolut über "/" aufgelöst
//     )
//   )

#import sys: inputs

#let customer-name = inputs.customer_name
#let generated-at = inputs.generated_at
#let sections = inputs.sections

#set document(title: customer-name + " – Wartungsdokumentation")
#set text(size: 10pt, lang: "de", font: "Libertinus Serif")
#set page(paper: "a4", margin: (top: 3.2cm, bottom: 2.8cm, x: 2.5cm))
#set par(justify: true)

// -- Deckblatt --------------------------------------------------------------
#align(center + horizon)[
  #text(28pt, weight: "bold")[Wartungsdokumentation]
  #v(1.5em)
  #text(18pt)[#customer-name]
  #v(3em)
  #text(11pt, fill: gray)[Erstellt am #generated-at]
]

#pagebreak()

// -- Inhaltsverzeichnis -------------------------------------------------------
#outline(title: "Inhaltsverzeichnis", depth: 2)

#pagebreak()

// -- Ab hier: Kopf-/Fußzeile mit Kundenname und Erstellungsdatum auf jeder Seite --
#set page(
  header: [
    #set text(size: 9pt, fill: gray)
    #customer-name
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

// -- Inhalt, gegliedert nach System -------------------------------------------
#for section in sections [
  = #section.system_name

  #for entry in section.entries [
    == #entry.title

    #text(size: 9pt, fill: gray)[
      #entry.category_label #sym.dot.c #entry.performed_at_display
      #if entry.late_entry_note != none [
        #sym.dot.c #entry.late_entry_note
      ]
    ]

    #if entry.tags.len() > 0 [
      #v(0.2em)
      #text(size: 8.5pt, fill: rgb("#2563eb"))[
        #entry.tags.map(t => "#" + t).join("  ")
      ]
    ]

    #v(0.6em)

    #block[
      #set heading(offset: 2)
      #eval(entry.body_typst, mode: "markup")
    ]

    #for path in entry.image_paths [
      #v(0.5em)
      #image("/" + path, width: 70%)
    ]

    #v(1.2em)
  ]
]
