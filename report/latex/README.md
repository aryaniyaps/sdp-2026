# LTME Project Report (LaTeX)

Main file: `authesis.tex` (AU thesis template, BTECH_2026 layout).

| File / folder | Contents |
|---|---|
| `Abstract/abstract.tex` | Abstract |
| `acknowl.tex` | Acknowledgement |
| `Acronyms/acronyms.tex` | List of abbreviations |
| `Glossary/glossary.tex` | Glossary of terms used in a specific sense (e.g. "memory") |
| `1/` … `5/` | Chapters 1 to 5 |
| `Conclusion/conclusion.tex` | Chapter 6, Conclusion and Future Work |
| `declaration_ai.tex` | Declaration on the use of AI tools |
| `Appendix/appendix.tex` | Appendix A (database schema) |
| `figures/` | All figures |
| `publication.bib` | References (`unsrt` style, as in the template) |

Build (TeX Live):

    latexmk -pdf authesis.tex

Without latexmk, run `pdflatex`, `bibtex authesis`, then `pdflatex` twice. `unsrt` ships with every TeX distribution.

Clean up build files with `latexmk -c`.

Custom helpers defined in `authesis.tex`:
- `\begin{maya} ... \end{maya}` is the shaded "Maya example" box. It never splits across pages, so keep everything for one example inside it. Set the title with `\begin{maya}[Maya example: what M1 saves]`. A table inside the box goes in a `minipage` with `\captionof{table}{...}`, not a `table` float. Put a figure that comes just before a box near the start of its section, so the float doesn't push the box off the page.
- `\code{...}` sets inline monospace text.
- `\begin{algorithm}` + `\begin{algorithmic}[1]` is a numbered algorithm in the template's format (`\State`, `\If`, `\For`).
- `\begin{datablock}` is a framed listing for example data (JSON, rows, config). It is single-spaced, wraps long lines inside the frame and never splits across pages.
- Section and subsection headings start a new page when fewer than a few lines of text would follow them, and long titles are left-aligned (patches to the class's `\AUsection`/`\AUsubsection`).
- `\begin{reporttable}` sets small, single-spaced table text. Use it around `tabularx`/`longtable`.
