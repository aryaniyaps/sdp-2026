# LTME Project Report (LaTeX)

Main file: `authesis.tex` (AU thesis template, BTECH_2026 layout).

| File / folder | Contents |
|---|---|
| `Abstract/abstract.tex` | Abstract |
| `acknowl.tex` | Acknowledgement |
| `Acronyms/acronyms.tex` | List of abbreviations |
| `Glossary/glossary.tex` | Glossary of terms used in a specific sense (e.g. "memory") |
| `1/` … `5/` | Chapters 1–5 |
| `figures/` | All figures |
| `publication.bib` | References (IEEEtran style) |

Build (TeX Live):

    latexmk -pdf authesis.tex

Without latexmk, run `pdflatex`, `bibtex authesis`, then `pdflatex` twice. The bibliography needs `IEEEtran.bst` (Debian/Ubuntu package `texlive-publishers`); if it is missing, bibtex reports "I couldn't open style file IEEEtran.bst" and the reference list comes out empty.

Clean up build files with `latexmk -c`.

Custom helpers defined in `authesis.tex`:
- `\begin{maya} ... \end{maya}` is the indented "Maya example." block. Change the label with `\begin{maya}[Module 1 output on Maya.]`.
- `\code{...}` sets inline monospace text.
- `\begin{algorithm}` + `\begin{pseudo}` is a numbered algorithm with a framed pseudocode box.
- `\begin{reporttable}` sets small, single-spaced table text. Use it around `tabularx`/`longtable`.
