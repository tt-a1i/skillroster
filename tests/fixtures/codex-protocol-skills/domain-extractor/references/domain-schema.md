# Domain glossary schema

The output is a JSON object with one key, `terms`. Each term is an object with
`id`, `label`, `definition`, and `relations`. Use lower-kebab IDs, copy labels
from the notes, keep definitions to one sentence, and use an empty array when
no relation is explicitly stated. `relations` contains objects with `type` and
`target` only. Preserve first appearance order and emit no additional keys.
