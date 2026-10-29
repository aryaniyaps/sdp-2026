# Live demo — 14 October 2026

Video backup: [Pi session memory demo](../video/pi-session-memory-demo.mp4). It plays offline, with Pi and the live graph side by side and no added text overlays.

Open [a fresh demo](http://127.0.0.1:18088/demo?namespace=guide%3Atomorrow-20261014&directory=scratch). Keep this namespace for the actual presentation; the rehearsal uses another namespace. If you repeat the presentation, choose a new namespace and click Apply, then New session. New session alone preserves memory, which is what the recall step needs.

The services are left running. Pi's GPT provider needs internet. Extraction, embeddings, the database and graph run locally. If the machine restarts, run `./deploy/azure/run-local.sh up` from the project folder. Run `./deploy/azure/run-local.sh check` before presenting. These commands use the prepared installation without builds or downloads.

## Demonstration script

Enter one message at a time in Pi. Wait for Pi to finish, then allow extraction to finish and the graph to refresh. A Pi acknowledgement is not proof that extraction is complete. Click the graph facts and source memories to show their supporting quotes.

1. `I spoke with Mira at Cedar Labs today about Project Orion. She told me that Project Orion uses PostgreSQL as its database and that she is the project owner.`
2. `I also spoke with Dev at Cedar Labs about Project Orion. Dev told me that Project Orion uses MySQL as its database.`
3. `Mira told me that Project Orion's database is MySQL.`
4. Show the conflicting database facts. Then enter: `Correction: Project Orion's database is SQLite, not PostgreSQL or MySQL. Mira checked the final deployment configuration and confirmed SQLite. Both earlier database reports are superseded.`
5. Show SQLite as active, earlier values as superseded, the source quotes and any derived observation. Click **New session** while keeping the same namespace.
6. Ask: `Who owns Project Orion, which database does it use now, and what happened to the earlier database reports? Answer from memory and cite the evidence.`

Wording and extraction coverage can vary with the model. Wait for the graph state before explaining a transition. If a provider is slow or unavailable during the presentation, use the verified video backup.

## Repairs and verification

- Review decoding now restricts correction evidence to the actual quoted source.
- An explicit supported correction can resolve an already contested value.
- Assistant answers influenced by recalled memory are retained with provenance and excluded as independent fact-extraction sources; new user and tool evidence remains eligible. This prevents recalled answers from feeding back as fresh contradictory facts.
- Regression coverage checks source-index mapping across windows with excluded evidence, Pi provenance across turns, and explicit recall.

Full verification results and the final live rehearsal receipt are stored beside this file. Changes are local and uncommitted.
