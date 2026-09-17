Publish current Harvest-native security scan's canonical result.
Call exactly once after every in-scope file and candidate reaches final disposition.
Evidence: only repository files inspected during this scan.
Tool: validates, fingerprints, assigns Harvest-owned IDs, writes canonical security store, creates SARIF.
NEVER invent IDs or edit store directly.
