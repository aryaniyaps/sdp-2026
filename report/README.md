# Project Report

- `latex/` is the report source. The main file is `latex/authesis.tex`, and `latex/LTME_Project_Report.pdf` is the latest compiled copy. Build and file notes are in `latex/README.md`.
- `diagrams/` holds the architecture diagrams as editable draw.io files (`*.drawio`). Each one also has a PNG export (`*.drawio.png`) with the diagram embedded, so the PNG opens in draw.io too.

## Updating a diagram

1. Edit the `.drawio` file in draw.io.
2. Export it: `drawio -x -f png -e -s 3 -o diagrams/<name>.drawio.png diagrams/<name>.drawio`
3. Copy the PNG over the matching figure in `latex/figures/`:

| Diagram | Report figure |
|---|---|
| `overall-architecture` | `fig3_1_architecture.png` |
| `module-1-source-acquisition` | `fig3_2_module1.png` |
| `module-2-knowledge-extraction` | `fig3_3_module2.png` |
| `module-3-temporal-graph` | `fig3_4_module3.png` |
| `module-4-hybrid-retrieval` | `fig3_5_module4.png` |
| `module-5-model-training` | `fig3_6_module5.png` |
