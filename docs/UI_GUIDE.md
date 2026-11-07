# 🖌️ NeuroStrata Visual Interface & Obsidian Guide

The NeuroStrata Obsidian plugin provides a native, seamless interface for curating and visualizing your AI's 3-Tier Dual-Temporal-Memory directly within your knowledge base. It connects securely to the NeuroStrata daemon (`http://127.0.0.1:34343`) to ensure your Obsidian notes and your AI's latent space remain perfectly synchronized.
NeuroStrata provides two complementary visual interfaces for exploring, auditing, and curating your AI agent's cognitive graph:
1. **The Embedded Web UI & Tauri Desktop App (`web-ui/` / `src-tauri/`):** An interactive 3D/2D Force Graph, Central Executive Task Board, SynapticMesh Inspector, and SPARQL/Digital-Twin console connected directly to the Rust daemon (`http://127.0.0.1:34343`).
2. **NeuroVault Obsidian Integration — Planned, not implemented:** Bidirectional Markdown note synchronization and JSON Canvas spatial topology export are tracked by task `neurostrata-dlag`; no `src/obsidian.rs` module exists in this release.

---

## 1. Interactive 3D & 2D SynapticGraph Canvas

The Web UI renders your project's exported graph (`.NeuroStrata/graph/graph.json` or live daemon stream) as an interactive spatial network:
- **Tri-Strata & Domain Filtering:** Filter nodes by namespace (`global` vs. project namespaces), memory type (`rule`, `fact`, `procedure`, `task`, `symbol`), or declarative domain shelf.
- **Directed Edge Inspection:** Visualize `GOVERNS`, `DEPENDS_ON`, `RELATES_TO`, `EXTRACTED_FROM`, and Tree-sitter AST (`calls`, `imports`) relationships in real time.
- **Leiden Community Coloring:** Color-code clusters discovered by NeuroStrata's Leiden community detection engine (`src/community.rs`) to spot architectural coupling at a glance.
- **Bi-Temporal Time Slider:** Inspect active memories (`valid_to == 0`) or toggle superseded and archived nodes (`--include-archived`) to audit how architectural rules evolved over time.

![Placeholder: NeuroStrata Inspector Sidebar Panel](./assets/neurostrata-inspector.png)
*(Drop your screenshot of the NeuroStrata Inspector sidebar here)*

**Key Features:**
* **Namespace Filtering:** Instantly filter memories by their 3 Tiers. View all `global` infrastructure mandates, or drill down into specific Domain rules (e.g., `nibble_server`) or Task contexts.
* **Live Semantic Search:** Type a natural language query into the inspector search bar to instantly find matching architectural rules or prior decisions across the database.
* **Full CRUD Control:** Did an agent hallucinate a rule? Did a project requirement change? You can edit the text of any memory directly in the sidebar, and the plugin will automatically re-embed and save it back to LadybugDB. You can also permanently delete obsolete memories with a single click.
### Refreshing the Graph Export
```bash
neurostrata-mcp export-graph
```

---

## 2. Central Executive Task Board & Gate Inspector

While agents can save memories autonomously, you can also easily inject human insight into the latent space using the editor's right-click context menu.

![Placeholder: Right-Click Context Menu in Obsidian Editor](./assets/neurostrata-right-click.png)
*(Drop your screenshot of the "Create NeuroStrata Memory (Paragraph)" context menu here)*

**Key Features:**
* **"Create NeuroStrata Memory (Paragraph)":** Simply right-click anywhere inside a narrative document (e.g., `docs/architecture/domains/ml_training.md`) and select this option.
* **Auto-Boundary Detection:** The plugin is intelligent enough to automatically detect the exact paragraph boundaries (from blank line to blank line) surrounding your cursor.
* **Structured Pointers (Pointer-Wiki):** It extracts the exact physical file path and line numbers (e.g., `lines 42-49`) and automatically attaches them as a structured JSON `refs` array inside the LadybugDB record. When an agent retrieves this memory later, it uses these coordinates to fetch only the necessary context window (Compact Reading).
The Task Board view surfaces the live state of the Central Executive (`src/task/`):
- **4-State Columns:** View `open`, `in_progress`, `blocked`, and `done` Goals alongside their active session claim leases and Breath history logs.
- **Lock 2 Extraction Links:** Click any `done` task to jump directly to the `rule` or `fact` nodes connected via `EXTRACTED_FROM` edges.
- **Gate Readiness Banner:** Displays the live `neurostrata-mcp task validate <namespace>` status (`violations`, `stale`, `dangling_edges`) so you know before pushing whether `.git/hooks/pre-push` will pass.

---

## 3. SynapticMesh, SPARQL & Digital Twin Panels

- **SynapticMesh Status:** Inspect queued outbox records, peer sync history, and any preserved conflict variants across your team's Git mesh (`docs/mesh/synaptic-mesh.md`).
- **SPARQL 1.1 & SHACL Console:** Run W3C SPARQL queries (`neurostrata_sparql_query`) and inspect SHACL shape conformance reports (`src/schemas/shacl/`).
- **Digital Twin Panel:** Inspect versioned state vectors, compare versions (`neurostrata_twin_diff`), and list the twins of a namespace (`neurostrata_twin_list`).

---

## 4. Local OS Editor Integration (Web UI)

If you are using the standalone browser-based **NeuroStrata Web UI** (via `/graph.json`), it includes a native file explorer to browse the physical codebase and the memory graph side-by-side. 

Because the web UI runs in a browser sandbox, we rely on custom OS URL protocols to launch your local editor.

**How it works:**
1. When the `neurostrata-mcp export-graph` command runs, it computes the absolute path for every file and memory node and injects it into the graph as `absolute_path`.
2. When you click a file in the Web UI's **File Explorer** (or click a memory node linked to a file), its details appear in the right-hand **Details & Viewer** panel.
3. You can select your preferred editor from the dropdown (VS Code, Cursor, or Obsidian) which is saved locally to your browser's `localStorage`.
4. Clicking **"Open File"** triggers the respective custom protocol:
   - **VS Code:** `vscode://file/{absolute_path}`
   - **Cursor:** `cursor://file/{absolute_path}`
   - **Obsidian:** `obsidian://open?path={encoded_absolute_path}`

---

## Getting Started

1. Ensure the **NeuroStrata daemon** is running in the background (it automatically starts when your AI client launches via MCP).
2. Enable the NeuroStrata Plugin in your Obsidian Community Plugins settings.
3. Open the command palette (`Ctrl+P` or `Cmd+P`) and type `NeuroStrata: Open Inspector` to launch the sidebar!
## 4. Obsidian Vault Synchronization (`NeuroVault`)

This feature is **not available in this release**. There is no `neurostrata_sync_obsidian` MCP tool, no `obsidian-sync` CLI subcommand, and no `src/obsidian.rs` module. The bidirectional Markdown sync and JSON Canvas export are planned and tracked by task `neurostrata-dlag`. See [`docs/ops/obsidian.md`](ops/obsidian.md) for the planned design and current status.
