<!-- durum: aktif -->
# Development tools

## Graphify

The local snapshot in graphify-out covers 107 code/config files and 26 documents
from the September 13 working tree. It contains 2,181 nodes, 6,374 edges and
79 communities. Generated assets, icons, dependency/build directories and the
maintainer's untracked idea list were excluded.

Open graphify-out/graph.html for the interactive view or GRAPH_REPORT.md for
the source-linked summary. Run from the repo root:

```powershell
graphify query 'StartupStore' --budget 1000
graphify explain 'Quarantine'
```

Use the graph to locate relevant files, then verify their current contents.
The first extraction reported 276 unresolved AST edge endpoints and repeated
relationships; generic types can also be linked across languages incorrectly.
See GRAPH_HEALTH.txt. This is an approximate navigation index, not a safety
audit or proof of a call path. Planned Phase 4 concepts are labelled as plans.

The Graphify benchmark estimated a 9.5× reduction against reading its entire
corpus, using generic built-in queries. This is a synthetic estimate, not a
measured saving for WinKnife work; one benchmark question concerns authentication,
which is not a representative task here. The explicit StartupStore query was
checked and found the store, page, API wrappers and relevant types.

Code extraction ran locally. Document extraction used two host-session helpers.
No separate paid API was used; their exact token usage was unavailable.
The graph is ignored by Git, because it is generated and contains local snapshot
metadata. Rebuild after substantial changes; source files remain authoritative.
The existing Graphify skill can perform incremental updates with its saved manifest.

## Additional setup decision

Keep existing Rust/Svelte tools and the new startup regression test. No new
paid dependency, persistent watcher, automatic Git hook, Obsidian export, or
mandatory graph-first agent rule was installed. The graph has demonstrated useful
lookup, but its unresolved relationships do not justify forcing every task through it.
Review/debug skills can be used when a task needs them; installing another skill
bundle would not resolve the outstanding release gates.
